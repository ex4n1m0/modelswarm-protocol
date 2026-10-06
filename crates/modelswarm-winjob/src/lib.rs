//! Windows FFI helpers for the desktop node: kill-on-close job objects
//! for supervised engine children, plus total-RAM detection. Kept as the
//! workspace's single unsafe-sanctioned crate.
//!
//! Force-killing the desktop app (taskkill /F, crash) must never orphan a
//! llama-server (observed live 2026-10-06). A job object created with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` ties every assigned process to the
//! job-handle's lifetime: when our process dies — however it dies — Windows
//! closes the handle and terminates the engine. The graceful shutdown path
//! still kills the child first; this is the belt to that braces.
//!
//! Job handles are retained for the process lifetime on purpose. This crate
//! is Windows-only by nature; it compiles to nothing useful elsewhere (all
//! entry points are `#[cfg(windows)]`).

#[cfg(windows)]
mod imp {
    use std::mem::size_of;
    use std::sync::{Mutex, OnceLock};
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(lpBuffer: *mut MemoryStatusEx) -> i32;
    }

    #[repr(C)]
    struct MemoryStatusEx {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }

    /// Total physical RAM in bytes (0 = unknown).
    pub fn total_ram_bytes() -> u64 {
        unsafe {
            let mut status: MemoryStatusEx = std::mem::zeroed();
            status.dw_length = std::mem::size_of::<MemoryStatusEx>() as u32;
            if GlobalMemoryStatusEx(&mut status) == 0 {
                return 0;
            }
            status.ull_total_phys
        }
    }

    /// Opaque kernel job handle; Send+Sync because it is only ever used by
    /// the FFI calls in this module.
    #[derive(Clone, Copy)]
    struct JobHandle(*mut core::ffi::c_void);
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    fn retained_jobs() -> &'static Mutex<Vec<JobHandle>> {
        static JOBS: OnceLock<Mutex<Vec<JobHandle>>> = OnceLock::new();
        JOBS.get_or_init(|| Mutex::new(Vec::new()))
    }

    /// Safe entry point: assigns `pid` (the engine child) to a
    /// kill-on-close job. Opens the process handle internally so no raw
    /// pointer crosses the crate boundary.
    pub fn assign_child(pid: u32) -> Result<(), String> {
        let process = open_process_for_job(pid)?;
        assign_kill_on_close_job(process)
    }

    /// Opens a process handle with the rights AssignProcessToJobObject needs.
    fn open_process_for_job(pid: u32) -> Result<*mut core::ffi::c_void, String> {
        unsafe {
            let handle = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if handle.is_null() {
                return Err(format!(
                    "OpenProcess({pid}): {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(handle)
        }
    }

    /// Creates a kill-on-close job, assigns `process_handle`, and retains
    /// the job handle until process death.
    pub fn assign_kill_on_close_job(process_handle: *mut core::ffi::c_void) -> Result<(), String> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(format!(
                    "CreateJobObjectW: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut core::ffi::c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                let e = format!(
                    "SetInformationJobObject: {}",
                    std::io::Error::last_os_error()
                );
                CloseHandle(job);
                return Err(e);
            }
            if AssignProcessToJobObject(job, process_handle) == 0 {
                let e = format!(
                    "AssignProcessToJobObject: {}",
                    std::io::Error::last_os_error()
                );
                CloseHandle(job);
                return Err(e);
            }
            retained_jobs()
                .lock()
                .map_err(|e| e.to_string())?
                .push(JobHandle(job));
            Ok(())
        }
    }

    /// Test hook: closes one retained job handle (KILL_ON_JOB_CLOSE then
    /// terminates its processes). Returns CloseHandle's result (1 = closed).
    pub fn close_one_retained_job_for_test() -> usize {
        let job = retained_jobs()
            .lock()
            .ok()
            .and_then(|mut v| v.pop())
            .map(|JobHandle(h)| h)
            .unwrap_or(std::ptr::null_mut());
        if job.is_null() {
            return 0;
        }
        unsafe { CloseHandle(job) as usize }
    }
}

#[cfg(windows)]
pub use imp::{assign_child, close_one_retained_job_for_test, total_ram_bytes};

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    fn job_handle_close_kills_assigned_child() {
        use std::process::Stdio;
        let child = std::process::Command::new("ping")
            .args(["-n", "60", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ping");
        let pid = child.id();
        super::assign_child(pid).expect("assign job");

        let pid_bytes = pid.to_string().into_bytes();
        let alive = || {
            std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}")])
                .output()
                .unwrap()
                .stdout
                .windows(pid_bytes.len())
                .any(|w| w == &pid_bytes[..])
        };
        assert!(alive(), "child must survive job assignment");

        assert_eq!(super::close_one_retained_job_for_test(), 1);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(!alive(), "closing the job handle must kill the child");
    }
}
