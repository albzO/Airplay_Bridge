//! 快速预留会话，后台准备、校验与发布；看门狗不等待驱动/磁盘锁。
//! Reserve immediately, prepare/validate/publish off-thread; the watchdog never waits on driver/disk locks.
use crate::{DesktopEvent, Engine, EventSink, Settings, auth, auth_memory, ensure_source, session};
use homepod_test::{
    capture,
    live::{self, GuiContext, GuiControl, GuiEmitter},
    privacy::Redactor,
    source::Source,
};
use serde_json::json;
use std::{
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

impl Engine {
    pub fn start_stream(
        self: &Arc<Self>,
        events: EventSink,
        names: Vec<String>,
        settings: Settings,
        password_first: bool,
    ) -> Result<u64, String> {
        settings.validate()?;
        if names.is_empty() || names.len() > 2 {
            return Err("请选择一台设备或一个立体声对".into());
        }
        let e = self.clone();
        let (id, control, preparation) = {
            let mut active = e.session.lock().unwrap();
            if e.quitting.load(Ordering::Acquire) {
                return Err("应用正在退出".into());
            }
            session::reserve(
                &mut active,
                e.configuration_busy.load(Ordering::Acquire)
                    || e.discovering.load(Ordering::Acquire),
                settings.mapping,
            )?
        };
        let watch_engine = e.clone();
        let watch_events = events.clone();
        let watch_control = control.clone();
        let watch_preparation = preparation.clone();
        let watch = move || {
            while watch_preparation.preparing() {
                let expired = {
                    let active = watch_engine.session.lock().unwrap();
                    if active.as_ref().map(|s| s.id) != Some(id) {
                        return;
                    }
                    let cancelled = watch_control.stop.load(Ordering::Acquire);
                    watch_preparation
                        .expire(&watch_control, Instant::now())
                        .map(|error| (error, cancelled))
                };
                if let Some((error, cancelled)) = expired {
                    if watch_preparation.claim_terminal() {
                        let error = homepod_test::failure::describe(error, "STREAM_FAILED");
                        let code = homepod_test::failure::code_for(&error, "STREAM_FAILED");
                        let _ = homepod_test::shutdown::run_until(
                            Instant::now() + Duration::from_millis(100),
                            move || {
                                watch_events(DesktopEvent::Stream {
                                    session_id: id,
                                    payload: json!({"kind":"finished","error":error,"safe_error":error,"error_code":code,"cancelled":cancelled}),
                                });
                            },
                        );
                    }
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
        };
        let watchdog = thread::Builder::new()
            .name("stream-prepare-watch".into())
            .spawn(watch);
        if let Err(error) = watchdog {
            preparation.finish();
            session::release(&mut e.session.lock().unwrap(), id);
            return Err(format!("无法启动准备监控线程：{error}"));
        }
        let worker_engine = e.clone();
        let worker_preparation = preparation.clone();
        let worker = thread::Builder::new()
            .name("stream-session".into())
            .spawn(move || {
                run(
                    events,
                    worker_engine,
                    id,
                    control,
                    worker_preparation,
                    names,
                    settings,
                    password_first,
                );
            });
        if let Err(error) = worker {
            preparation.finish();
            session::release(&mut e.session.lock().unwrap(), id);
            return Err(format!("无法启动会话线程：{error}"));
        }
        Ok(id)
    }
}

fn run(
    events: EventSink,
    e: Arc<Engine>,
    id: u64,
    control: Arc<GuiControl>,
    preparation: Arc<session::Preparation>,
    names: Vec<String>,
    settings: Settings,
    password_first: bool,
) {
    let check = || {
        preparation.check(&control)?;
        if e.quitting.load(Ordering::Acquire) {
            return Err("已取消建立流".into());
        }
        if e.session.lock().unwrap().as_ref().map(|s| s.id) != Some(id) {
            return Err("已取消建立流".into());
        }
        Ok(())
    };
    let mut server: Option<auth::Server> = None;
    let mut policy_worker: Option<auth_memory::Worker> = None;
    let mut policy_events: Option<auth_memory::Sender> = None;
    let mut owned_source: Option<Arc<Source>> = None;
    let mut activated = false;
    let mut privacy = Redactor::default();
    let stream_events = events.clone();
    let emit: GuiEmitter = Arc::new(move |payload| {
        stream_events(DesktopEvent::Stream {
            session_id: id,
            payload,
        });
    });
    emit(json!({"kind":"preparing_source"}));
    let work = || -> Result<(), String> {
        check()?;
        if !e.backend.exists() {
            return Err("缺少 airplay-backend.exe，请从完整 dist 文件夹启动".into());
        }
        if names.is_empty() || names.len() > 2 {
            return Err("请选择一台设备或一个立体声对".into());
        }
        settings.validate()?;
        let found = e.devices();
        let mut chosen = Vec::new();
        for name in &names {
            let d: Vec<_> = found.iter().filter(|d| &d.name == name).collect();
            if d.len() != 1 {
                return Err("设备名称不唯一，请刷新设备".into());
            }
            chosen.push(d[0]);
        }
        if names.len() == 2
            && (names[0] == names[1]
                || chosen[0]
                    .properties
                    .get("tsid")
                    .filter(|v| !v.is_empty())
                    .is_none()
                || chosen[0].properties.get("tsid") != chosen[1].properties.get("tsid"))
        {
            return Err("这两台设备不是同一个立体声对".into());
        }
        let hosts: Vec<String> = chosen
            .iter()
            .map(|d| {
                d.addresses
                    .first()
                    .map(|a| a.to_string())
                    .ok_or("设备没有 IPv4 地址".to_string())
            })
            .collect::<Result<_, _>>()?;
        let policies = e.auth_memory.lock().unwrap();
        let password_first = password_first || policies.needs_password(chosen[0]);
        let peer_password_first = chosen.get(1).is_some_and(|d| policies.needs_password(d));
        drop(policies);
        let identities: Vec<_> = hosts
            .iter()
            .cloned()
            .zip(chosen.iter().map(|d| auth_memory::key(d)))
            .collect();
        let endpoint = settings.endpoint.clone();
        check()?;
        let inputs =
            preparation.step(&control, || capture::enumerate().map_err(|e| e.to_string()))?;
        check()?;
        let input = inputs
            .iter()
            .find(|i| i.id == endpoint)
            .ok_or("请选择有效的音频来源")?;
        let channels = input.channels.ok_or("不支持此采集格式")? as usize;
        if settings.mapping.iter().any(|i| *i >= channels) {
            return Err("声道映射超出了输入设备的声道数".into());
        }

        privacy = Redactor::new(&found);
        settings.save_checked(&e.root, check)?;
        *e.settings.lock().unwrap() = settings.clone();
        check()?;
        let event_emit = emit.clone();
        let policy_engine = e.clone();
        let worker = auth_memory::Worker::start(identities, control.clone(), move |identity| {
            let mut next = policy_engine.auth_memory.lock().unwrap().clone();
            next.remember(&policy_engine.root, identity)?;
            *policy_engine.auth_memory.lock().unwrap() = next;
            Ok(())
        })
        .map_err(|error| format!("[AUTH_CACHE_FAILED] 认证策略后台线程启动失败：{error}"))?;
        let policies = worker.sender();
        policy_events = Some(policies.clone());
        policy_worker = Some(worker);
        let emit: GuiEmitter = Arc::new(move |event| {
            policies.observe(&event);
            for error in policies.take_errors() {
                event_emit(
                    json!({"kind":"auth_memory_error","error":format!("认证方式记录保存失败：{error}")}),
                );
            }
            event_emit(event);
        });

        // 回调只登记有界请求/取错误；认证策略锁和文件操作仅在后台线程。
        // The callback only queues bounded requests/drains errors; policy locks and disk stay off it.
        let prepared_emit = emit;
        let pipe = format!("\\\\.\\pipe\\airplay-bridge-{}-{id}", std::process::id());
        server = Some(auth::Server::start(
            pipe.clone(),
            control.stop.clone(),
            hosts.clone(),
            prepared_emit.clone(),
        )?);
        server
            .as_mut()
            .unwrap()
            .use_shutdown_budget(control.shutdown.clone());
        check()?;
        let (source, created) = ensure_source(events.clone(), &e, &settings, check)?;
        if created {
            owned_source = Some(source.clone());
        }
        source
            .wait_ready(&control.stop)
            .map_err(|e| e.to_string())?;
        check()?;
        control.speakers_swapped.store(
            settings.speakers_swapped && names.len() == 2,
            Ordering::Release,
        );
        {
            let mut active_source = e.source.lock().unwrap();
            let mut active = e.session.lock().unwrap();
            let active = active
                .as_mut()
                .filter(|s| s.id == id)
                .ok_or("已取消建立流")?;
            if e.quitting.load(Ordering::Acquire) {
                return Err("已取消建立流".into());
            }
            preparation.activate(&control)?;
            if created {
                *active_source = Some(source.clone());
            }
            active.reply = Some(server.as_ref().unwrap().replies.clone());
            active.hosts = hosts;
            e.capture_enabled.store(true, Ordering::Release);
        }
        activated = true;
        // 唤醒失败只提示，不中断串流。
        // A failed power request is reported without interrupting the stream.
        if let Err(error) = e.sync_awake() {
            prepared_emit(
                json!({"kind":"awake_error","error":homepod_test::failure::describe(&error, "AWAKE_FAILED")}),
            );
        }
        events(DesktopEvent::SourceLevel(
            json!({"endpoint":endpoint,"captureEnabled":true}),
        ));
        let context = GuiContext {
            backend: e.backend.clone(),
            endpoint,
            source,
            password_pipe: pipe,
            password_first,
            peer_password_first,
            detailed_logs: settings.detailed_logs,
            capture_diagnostics: settings.capture_diagnostics,
            control: control.clone(),
            emit: prepared_emit,
        };
        live::run_gui(
            &e.root,
            &names[0],
            names.get(1).map(String::as_str),
            settings.latency,
            settings.buffer,
            context,
        )
        .map_err(|e| e.to_string())
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
        .unwrap_or_else(|_| Err("会话工作线程异常".into()));
    let mut result =
        result.map_err(|detail| homepod_test::failure::describe(&detail, "STREAM_FAILED"));
    if result
        .as_ref()
        .is_err_and(|error| error.contains("[STOP_TIMEOUT]"))
    {
        control.cleanup_complete.store(false, Ordering::Release);
    }
    let cancelled = control.stop.load(Ordering::Acquire);
    let deadline = control.shutdown.begin();
    if !activated {
        if let Some(source) = owned_source {
            if let Err(error) = source.stop_until(deadline - Duration::from_millis(100)) {
                control.cleanup_complete.store(false, Ordering::Release);
                result = Err(format!("{}；{error}", result.err().unwrap_or_default()));
            } else {
                let mut active = e.source.lock().unwrap();
                if active.as_ref().is_some_and(|s| Arc::ptr_eq(s, &source)) {
                    *active = None;
                }
            }
        }
    }
    if let Some(mut server) = server {
        if !server.shutdown_until(deadline - Duration::from_millis(50)) {
            control.cleanup_complete.store(false, Ordering::Release);
            if result.is_ok() {
                result = Err("[STOP_TIMEOUT] 密码管道收尾超过统一停止期限".into());
            }
        }
        drop(server);
    }
    if let Some(mut worker) = policy_worker {
        if let Err(error) = worker.finish_until(deadline - Duration::from_millis(100)) {
            let primary = result
                .err()
                .map(|error| format!("{error}；"))
                .unwrap_or_default();
            result = Err(format!("{primary}认证策略收尾：{error}"));
            if worker.unfinished() {
                if let Some(active) = e.session.lock().unwrap().as_mut().filter(|s| s.id == id) {
                    active.auth_policy = Some(worker);
                }
            }
        }
    }
    if let Some(policies) = policy_events {
        let errors = policies.take_errors();
        if !errors.is_empty() {
            let notify = emit.clone();
            let error = format!("认证方式记录保存失败：{}", errors.join("；"));
            if let Err(error) = homepod_test::shutdown::run_until(
                deadline.min(Instant::now() + Duration::from_millis(50)),
                move || notify(json!({"kind":"auth_memory_error","error":error})),
            ) {
                control.cleanup_complete.store(false, Ordering::Release);
                let primary = result
                    .err()
                    .map(|error| format!("{error}；"))
                    .unwrap_or_default();
                result = Err(format!("{primary}{error}"));
            }
        }
    }
    // 先确认资源收尾，再标记完成/释放预留。超时占用不由迟到结果清除。
    // Confirm cleanup before completion/release; late results never clear an expired reservation.
    let notify = preparation.finish();
    session::release(&mut e.session.lock().unwrap(), id);
    let _ = e.sync_awake();
    if !notify {
        return;
    }
    let error = finished_error(
        result,
        cancelled,
        control.cleanup_complete.load(Ordering::Acquire),
    );
    if !cancelled {
        if let Some(error) = &error {
            let root = e.root.clone();
            let safe = privacy.text(error);
            let _ = homepod_test::shutdown::run_until(
                deadline.min(Instant::now() + Duration::from_millis(100)),
                move || {
                    let _ = homepod_test::log_store::write(
                        &root.join("logs").join(format!("ui-fault-{id}.log")),
                        safe,
                        64 * 1024,
                    );
                },
            );
        }
    }
    let safe_error = error.as_ref().map(|error| privacy.text(error));
    let error_code = error
        .as_ref()
        .map(|error| homepod_test::failure::code_for(error, "STREAM_FAILED"));
    let _ = homepod_test::shutdown::run_until(deadline, move || {
        emit(
            json!({"kind":"finished","error":error,"safe_error":safe_error,"error_code":error_code,"cancelled":cancelled}),
        );
    });
}

fn finished_error(
    result: Result<(), String>,
    cancelled: bool,
    cleanup_complete: bool,
) -> Option<String> {
    let original = result.err();
    if !cleanup_complete {
        return Some(format!(
            "{}[STOP_TIMEOUT] 旧会话资源尚未确认退出，请重启应用后连接",
            original.map_or(String::new(), |error| format!("{error}；"))
        ));
    }
    original.filter(|error| {
        !cancelled
            || !(error.ends_with("详情：已取消建立流") || error.ends_with("详情：启动采集已取消"))
    })
}

#[cfg(test)]
#[path = "../../test/frontend/desktop/shutdown.rs"]
mod shutdown_checks;
