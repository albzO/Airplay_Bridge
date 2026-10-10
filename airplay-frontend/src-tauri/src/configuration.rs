//! 后台配置/预览/发现互斥。占用登记只持有短会话锁，慢操作不持有全局会话/设置锁。
//! Admit one background configuration/preview/discovery operation under a short session lock.
use crate::{Engine, Settings, devices, ensure_source, routing, stop_source};
use homepod_test::{capture, discovery};
use serde_json::json;
use std::sync::{Arc, atomic::Ordering};
use tauri::{Emitter, State};

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
    app: tauri::AppHandle,
    e: &Engine,
    settings: &Settings,
    operation: &Operation,
) -> Result<Arc<homepod_test::source::Source>, String> {
    let (source, created) = ensure_source(app, e, settings, || operation.check())?;
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

#[tauri::command]
pub async fn discover_devices(
    engine: State<'_, Arc<Engine>>,
) -> Result<Vec<discovery::Device>, String> {
    let e = engine.inner().clone();
    let operation = Operation::begin(e.clone(), false)?;
    tauri::async_runtime::spawn_blocking(move || {
        operation.check()?;
        discovery::discover(&e.root, 6).map_err(|e| e.to_string())?;
        operation.check()?;
        Ok(devices(&e))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn monitor_source(
    app: tauri::AppHandle,
    engine: State<'_, Arc<Engine>>,
) -> Result<(), String> {
    let e = engine.inner().clone();
    let operation = Operation::begin(e.clone(), false)?;
    tauri::async_runtime::spawn_blocking(move || {
        if !e.capture_enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        let settings = e.settings.lock().unwrap().clone();
        let source = preview_source(app, &e, &settings, &operation)?;
        source.wait_ready(&e.quitting).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_capture_enabled(
    app: tauri::AppHandle,
    engine: State<'_, Arc<Engine>>,
    enabled: bool,
) -> Result<(), String> {
    let e = engine.inner().clone();
    let operation = Operation::begin(e.clone(), false)?;
    tauri::async_runtime::spawn_blocking(move || {
        operation.check()?;
        let settings = e.settings.lock().unwrap().clone();
        if enabled {
            preview_source(app.clone(), &e, &settings, &operation)?;
        } else {
            stop_source(&e)?;
        }
        operation.check()?;
        e.capture_enabled.store(enabled, Ordering::Release);
        let _ = app.emit(
            "source-level",
            json!({"endpoint":settings.endpoint,"captureEnabled":enabled,"peaks":[0.,0.]}),
        );
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn save_settings(
    engine: State<'_, Arc<Engine>>,
    settings: Settings,
) -> Result<(), String> {
    settings.validate()?;
    let e = engine.inner().clone();
    let operation = Operation::begin(e.clone(), true)?;
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_mapping(
    engine: State<'_, Arc<Engine>>,
    mapping: [usize; 2],
) -> Result<(), String> {
    let e = engine.inner().clone();
    let operation = Operation::begin(e.clone(), true)?;
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_speaker_order(
    engine: State<'_, Arc<Engine>>,
    swapped: bool,
) -> Result<(), String> {
    let e = engine.inner().clone();
    let operation = Operation::begin(e.clone(), true)?;
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}
