use super::*;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize,
};

#[test]
fn wasapi_job_can_initialize_sta_when_the_caller_is_mta() {
    thread::spawn(|| {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap() };
        let result = dispatch(Box::new(|| {
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                    .ok()
                    .map_err(|error| error.to_string())?;
                CoUninitialize();
            }
            Ok(())
        }))
        .unwrap()
        .recv_blocking()
        .unwrap();
        unsafe { CoUninitialize() };
        assert_eq!(result, Ok(()));
    })
    .join()
    .unwrap();
}
