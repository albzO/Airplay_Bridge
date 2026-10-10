//! 会话预留与准备期限；不在这些锁内调用驱动、文件或线程等待。
//! Session reservation and preparation deadline; no driver, file or join calls under these locks.
use crate::auth;
use homepod_test::live::GuiControl;
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const PREPARE_TIMEOUT: Duration = Duration::from_secs(15);
static NEXT_ID: OnceLock<AtomicU64> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    Active,
    Expired,
    Done,
}

pub struct Preparation {
    phase: Mutex<Phase>,
    deadline: Instant,
    terminal: AtomicBool,
}

impl Preparation {
    pub fn new(deadline: Instant) -> Self {
        Self {
            phase: Mutex::new(Phase::Preparing),
            deadline,
            terminal: AtomicBool::new(false),
        }
    }

    pub fn preparing(&self) -> bool {
        *self.phase.lock().unwrap() == Phase::Preparing
    }

    pub fn check(&self, control: &GuiControl) -> Result<(), String> {
        self.check_at(control, Instant::now())
    }

    pub fn step<T>(
        &self,
        control: &GuiControl,
        work: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.check(control)?;
        let result = work()?;
        self.check(control)?;
        Ok(result)
    }

    fn check_at(&self, control: &GuiControl, now: Instant) -> Result<(), String> {
        if matches!(*self.phase.lock().unwrap(), Phase::Expired) || now >= self.deadline {
            return Err("[START_TIMEOUT] 启动准备超过 15 秒".into());
        }
        if control.stop.load(Ordering::Acquire) {
            return Err("已取消建立流".into());
        }
        Ok(())
    }

    /// 与 expire 共用阶段锁，超时后的迟到结果不能发布为 Active。
    /// Share the phase lock with expire so late results cannot activate an expired request.
    pub fn activate(&self, control: &GuiControl) -> Result<(), String> {
        let mut phase = self.phase.lock().unwrap();
        if *phase != Phase::Preparing || Instant::now() >= self.deadline {
            return Err("[START_TIMEOUT] 启动准备超过 15 秒".into());
        }
        if control.stop.load(Ordering::Acquire) {
            return Err("已取消建立流".into());
        }
        *phase = Phase::Active;
        Ok(())
    }

    /// 看门狗只观察准备阶段；阻塞工作未确认退出时保留会话，禁止累积重试。
    /// Only supervise preparation; retain occupancy when a blocked worker has not confirmed exit.
    pub fn expire(&self, control: &GuiControl, now: Instant) -> Option<&'static str> {
        let mut phase = self.phase.lock().unwrap();
        if *phase != Phase::Preparing {
            return None;
        }
        let stop_expired = control
            .shutdown
            .current()
            .is_some_and(|deadline| now >= deadline);
        if now < self.deadline && !stop_expired {
            return None;
        }
        *phase = Phase::Expired;
        control.cleanup_complete.store(false, Ordering::Release);
        control.request_stop();
        Some(if stop_expired {
            "[STOP_TIMEOUT] 启动准备尚未确认退出，请重启应用后连接"
        } else {
            "[START_TIMEOUT] 启动准备超过 15 秒，后台操作尚未确认退出，请重启应用后连接"
        })
    }

    pub fn finish(&self) -> bool {
        *self.phase.lock().unwrap() = Phase::Done;
        self.claim_terminal()
    }

    pub fn claim_terminal(&self) -> bool {
        !self.terminal.swap(true, Ordering::AcqRel)
    }
}

pub struct Session {
    pub id: u64,
    pub control: Arc<GuiControl>,
    pub preparation: Arc<Preparation>,
    pub reply: Option<Sender<auth::Reply>>,
    pub hosts: Vec<String>,
}

pub fn reserve(
    active: &mut Option<Session>,
    configuration_busy: bool,
    mapping: [usize; 2],
) -> Result<(u64, Arc<GuiControl>, Arc<Preparation>), String> {
    if active
        .as_ref()
        .is_some_and(|s| !s.control.cleanup_complete.load(Ordering::Acquire))
    {
        return Err("[STOP_TIMEOUT] 上次会话资源尚未确认退出，请重启应用后连接".into());
    }
    if active.is_some() || configuration_busy {
        return Err("[OPERATION_BUSY] 已有会话或正在修改配置/发现设备".into());
    }
    // 首次以毫秒时间播种，之后只递增；时钟倒退不复用编号，日志保留时间编号。
    // Seed once from epoch milliseconds, then increment independently of wall-clock changes.
    let id = NEXT_ID
        .get_or_init(|| {
            AtomicU64::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            )
        })
        .fetch_add(1, Ordering::Relaxed);
    let control = Arc::new(GuiControl::new(mapping));
    let preparation = Arc::new(Preparation::new(Instant::now() + PREPARE_TIMEOUT));
    *active = Some(Session {
        id,
        control: control.clone(),
        preparation: preparation.clone(),
        reply: None,
        hosts: Vec::new(),
    });
    Ok((id, control, preparation))
}

pub fn stop(active: &Option<Session>, id: u64) {
    if let Some(session) = active.as_ref().filter(|s| s.id == id) {
        session.control.request_stop();
    }
}

pub fn release(active: &mut Option<Session>, id: u64) {
    if active
        .as_ref()
        .is_some_and(|s| s.id == id && s.control.cleanup_complete.load(Ordering::Acquire))
    {
        *active = None;
    }
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/session.rs"]
mod tests;
