//! 先保存路由，再发布给采集和会话控制面。
//! Persist routing before publishing it to the capture/session control plane.
use crate::settings::Settings;
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub fn set_mapping(
    root: &Path,
    settings: &mut Settings,
    session: Option<&Mutex<[usize; 2]>>,
    source: Option<&Mutex<[usize; 2]>>,
    mapping: [usize; 2],
) -> Result<(), String> {
    let mut next = settings.clone();
    next.mapping = mapping;
    next.save(root)?;
    // 保存失败时，不改变内存设置和正在播放的声道。
    // A failed save leaves both remembered and live routing unchanged.
    *settings = next;
    if let Some(mapping_state) = session {
        *mapping_state.lock().unwrap() = mapping;
    }
    if let Some(mapping_state) = source {
        *mapping_state.lock().unwrap() = mapping;
    }
    Ok(())
}

pub fn set_speaker_order(
    root: &Path,
    settings: &mut Settings,
    active: Option<&AtomicBool>,
    swapped: bool,
) -> Result<(), String> {
    let mut next = settings.clone();
    next.speakers_swapped = swapped;
    next.save(root)?;
    *settings = next;
    if let Some(active) = active {
        active.store(swapped, Ordering::Relaxed);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../test/frontend/desktop/routing.rs"]
mod tests;
