//! 后台配置/预览/发现互斥。占用登记只持有短会话锁，慢操作不持有全局会话/设置锁。
//! Admit one background configuration/preview/discovery operation under a short session lock.
use crate::{
    DesktopEvent, DesktopJob, Engine, EventSink, Settings, ensure_source, routing, stop_source,
};
use homepod_test::{capture, discovery};
use serde_json::json;
use std::sync::{Arc, atomic::Ordering};

pub struct Operation {
    engine: Arc<Engine>,
    session_id: Option<u64>,
}

impl Operation {
    pub fn begin(engine: Arc<Engine>, allow_active: bool) -> Result<Self, String> {
        let session = engine.session.lock().unwrap();
        if engine.quitting.load(Ordering::Acquire) {
            return Err("应用正在退出".into());
        }
        if session.as_ref().is_some_and(|s| {
            !allow_active
                || s.preparation.preparing()
                || !s.control.cleanup_complete.load(Ordering::Acquire)
        }) {
            return Err("[OPERATION_BUSY] 请等待启动准备结束或停止串流后再操作".into());
        }
        if engine.configuration_busy.swap(true, Ordering::AcqRel) {
            return Err("[OPERATION_BUSY] 正在修改配置/采集或发现设备，请稍后重试".into());
        }
        let session_id = session.as_ref().map(|s| s.id);
        drop(session);
        Ok(Self { engine, session_id })
    }

    pub fn check(&self) -> Result<(), String> {
        if self.engine.quitting.load(Ordering::Acquire) {
            return Err("应用正在退出".into());
        }
        let session = self.engine.session.lock().unwrap();
        if session.as_ref().map(|s| s.id) != self.session_id
            || session
                .as_ref()
                .is_some_and(|s| s.control.stop.load(Ordering::Acquire))
        {
            return Err("会话已结束，请重新操作".into());
        }
        Ok(())
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        self.engine
            .configuration_busy
            .store(false, Ordering::Release);
    }
}

fn preview_source(
    events: EventSink,
    e: &Engine,
    settings: &Settings,
    operation: &Operation,
) -> Result<Arc<homepod_test::source::Source>, String> {
    let (source, created) = ensure_source(events, e, settings, || operation.check())?;
    if created {
        let result = {
            let mut active = e.source.lock().unwrap();
            let result = operation.check();
            if result.is_ok() {
                *active = Some(source.clone());
            }
            result
        };
        if let Err(error) = result {
            if let Err(cleanup) = source.stop() {
                // 未确认停止时保留句柄，后续 ensure_source 不会另建并发采集器。
                // Retain an unconfirmed source so a later ensure_source cannot open a second one.
                *e.source.lock().unwrap() = Some(source);
                return Err(format!("{error}；{cleanup}"));
            }
            return Err(error);
        }
    }
    Ok(source)
}

impl Engine {
    pub fn prepare_discover_devices(
        self: &Arc<Self>,
    ) -> Result<DesktopJob<Vec<discovery::Device>>, String> {
        let e = self.clone();
        let operation = Operation::begin(e.clone(), false)?;
        Ok(Box::new(move || {
            operation.check()?;
            discovery::discover(&e.root, 6).map_err(|e| e.to_string())?;
            operation.check()?;
            Ok(e.devices())
        }))
    }

    pub fn prepare_monitor_source(
        self: &Arc<Self>,
        events: EventSink,
    ) -> Result<DesktopJob<()>, String> {
        let e = self.clone();
        let operation = Operation::begin(e.clone(), false)?;
        Ok(Box::new(move || {
            if !e.capture_enabled.load(Ordering::Acquire) {
                return Ok(());
            }
            let settings = e.settings.lock().unwrap().clone();
            let source = preview_source(events, &e, &settings, &operation)?;
            source.wait_ready(&e.quitting).map_err(|e| e.to_string())
        }))
    }

    pub fn prepare_set_capture_enabled(
        self: &Arc<Self>,
        events: EventSink,
        enabled: bool,
    ) -> Result<DesktopJob<()>, String> {
        let e = self.clone();
        let operation = Operation::begin(e.clone(), false)?;
        Ok(Box::new(move || {
            operation.check()?;
            let settings = e.settings.lock().unwrap().clone();
            if enabled {
                preview_source(events.clone(), &e, &settings, &operation)?;
            } else {
                stop_source(&e)?;
            }
            operation.check()?;
            e.capture_enabled.store(enabled, Ordering::Release);
            events(DesktopEvent::SourceLevel(
                json!({"endpoint":settings.endpoint,"captureEnabled":enabled,"peaks":[0.,0.]}),
            ));
            Ok(())
        }))
    }

    pub fn prepare_save_settings(
        self: &Arc<Self>,
        settings: Settings,
    ) -> Result<DesktopJob<()>, String> {
        settings.validate()?;
        let e = self.clone();
        let operation = Operation::begin(e.clone(), true)?;
        Ok(Box::new(move || {
            operation.check()?;
            let was_active = {
                let mut awake = e.awake.lock().unwrap();
                operation.check()?;
                let previous = awake.active();
                awake.set(settings.keep_awake)?;
                previous
            };
            if let Err(error) = settings.save_checked(&e.root, || operation.check()) {
                // 退出已经释放电源请求时，迟到失败不能重新获取它。
                // Do not reacquire power requests after quit has released them.
                let mut awake = e.awake.lock().unwrap();
                if !e.quitting.load(Ordering::Acquire) {
                    let _ = awake.set(was_active);
                }
                return Err(error);
            }
            // rename 成功是设置的提交点；即使此刻收到停止，也保持内存与磁盘一致。
            // A successful rename commits preferences, including a concurrent stop at that boundary.
            *e.settings.lock().unwrap() = settings;
            Ok(())
        }))
    }

    pub fn prepare_set_mapping(
        self: &Arc<Self>,
        mapping: [usize; 2],
    ) -> Result<DesktopJob<()>, String> {
        let e = self.clone();
        let operation = Operation::begin(e.clone(), true)?;
        Ok(Box::new(move || {
            operation.check()?;
            let mut settings = e.settings.lock().unwrap().clone();
            let inputs = capture::enumerate().map_err(|e| e.to_string())?;
            operation.check()?;
            let channels = inputs
                .iter()
                .find(|i| i.id == settings.endpoint)
                .and_then(|i| i.channels)
                .ok_or("无有效采集设备")? as usize;
            if mapping.iter().any(|c| *c >= channels) {
                return Err("声道超出范围".into());
            }
            let control = e
                .session
                .lock()
                .unwrap()
                .as_ref()
                .map(|s| s.control.clone());
            let source = e.source.lock().unwrap().clone();
            routing::set_mapping(
                &e.root,
                &mut settings,
                control.as_ref().map(|s| &s.mapping),
                source.as_ref().map(|s| &s.mapping),
                mapping,
                || operation.check(),
            )?;
            *e.settings.lock().unwrap() = settings;
            Ok(())
        }))
    }

    pub fn prepare_set_speaker_order(
        self: &Arc<Self>,
        swapped: bool,
    ) -> Result<DesktopJob<()>, String> {
        let e = self.clone();
        let operation = Operation::begin(e.clone(), true)?;
        Ok(Box::new(move || {
            operation.check()?;
            let control = {
                let session = e.session.lock().unwrap();
                if session.as_ref().is_some_and(|s| s.hosts.len() != 2) {
                    return Err("仅立体声对可交换扬声器".into());
                }
                session.as_ref().map(|s| s.control.clone())
            };
            let mut settings = e.settings.lock().unwrap().clone();
            routing::set_speaker_order(
                &e.root,
                &mut settings,
                control.as_ref().map(|s| &s.speakers_swapped),
                swapped,
                || operation.check(),
            )?;
            *e.settings.lock().unwrap() = settings;
            Ok(())
        }))
    }

    pub fn prepare_forget_auth_policy(
        self: &Arc<Self>,
        names: Vec<String>,
    ) -> Result<DesktopJob<()>, String> {
        let e = self.clone();
        let operation = Operation::begin(e.clone(), false)?;
        Ok(Box::new(move || {
            operation.check()?;
            let selected: Vec<_> = e
                .devices()
                .into_iter()
                .filter(|d| names.contains(&d.name))
                .collect();
            let mut next = e.auth_memory.lock().unwrap().clone();
            operation.check()?;
            next.forget(&e.root, &selected)?;
            // 文件替换已经提交，即使退出已开始也要同步内存。
            // Disk replacement committed; publish the same state even if quitting began meanwhile.
            *e.auth_memory.lock().unwrap() = next;
            Ok(())
        }))
    }
}
