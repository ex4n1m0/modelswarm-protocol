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
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_MORE_DATA};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicProcessIdList,
        JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
        JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
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

    /// PIDs currently assigned to this process's retained kill-on-close
    /// jobs — job-object MEMBERSHIP, i.e. exactly the supervised engine
    /// set (children inherit job membership on creation, so anything an
    /// engine spawned is included too). This is the resource monitor
    /// panel's scoping query (the sampler handoff 2026-10-12 §4 option
    /// (a): the panel's process tree equals the supervised set, immune to
    /// PID-reuse orphans). A job whose query fails is skipped — an honest
    /// per-job undercount, never a failure of the whole call.
    pub fn supervised_process_ids() -> Vec<u32> {
        let jobs = match retained_jobs().lock() {
            Ok(jobs) => jobs,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        for &JobHandle(job) in jobs.iter() {
            out.extend(job_process_ids(job));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// One job's member PIDs via `QueryInformationJobObject(
    /// JobObjectBasicProcessIdList)`. Starts with a 64-slot buffer (each
    /// job holds one engine child plus its own descendants — dozens is
    /// already exotic), doubles once on `ERROR_MORE_DATA` up to 1024, and
    /// returns empty on any other failure (a dead/raced handle reads as
    /// "no members", not as a panic).
    fn job_process_ids(job: *mut core::ffi::c_void) -> Vec<u32> {
        let mut capacity = 64usize;
        loop {
            // Layout: two u32 counters (one usize word) + capacity PIDs.
            let words = 1 + capacity;
            let mut buf: Vec<usize> = vec![0; words];
            let mut returned: u32 = 0;
            let ok = unsafe {
                QueryInformationJobObject(
                    job,
                    JobObjectBasicProcessIdList,
                    buf.as_mut_ptr().cast(),
                    (words * size_of::<usize>()) as u32,
                    &mut returned,
                )
            };
            if ok == 0 {
                let more_data =
                    std::io::Error::last_os_error().raw_os_error() == Some(ERROR_MORE_DATA as i32);
                if more_data && capacity < 1024 {
                    capacity *= 2;
                    continue;
                }
                return Vec::new();
            }
            let list = unsafe { &*(buf.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST) };
            let count = list.NumberOfProcessIdsInList as usize;
            if count > capacity {
                return Vec::new();
            }
            let ids = unsafe { std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count) };
            return ids.iter().map(|&pid| pid as u32).collect();
        }
    }
}

#[cfg(windows)]
pub use imp::{
    assign_child, close_one_retained_job_for_test, supervised_process_ids, total_ram_bytes,
};

#[cfg(test)]
mod tests {
    /// Both job tests share ONE retained-jobs stack, and
    /// `close_one_retained_job_for_test` pops LIFO: if they interleaved,
    /// one test's close could pop (and kill) the OTHER test's child. This
    /// lock serializes them for the whole assign→assert→cleanup span.
    fn job_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    #[cfg(windows)]
    fn job_handle_close_kills_assigned_child() {
        use std::process::Stdio;
        let _guard = job_test_lock();
        let mut child = std::process::Command::new("ping")
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
        // Reap the killed child so the test leaves no zombie behind
        // (clippy::zombie_processes; the job kill delivers the exit).
        let _ = child.wait();
    }

    /// Job-object membership (the resource monitor panel's scoping query,
    /// sampler handoff §4 option (a)): an assigned child is reported by
    /// `supervised_process_ids`. Cleanup is a direct child kill + reap —
    /// `close_one_retained_job_for_test` pops an arbitrary retained job
    /// (tests run concurrently) and could kill ANOTHER test's child.
    #[test]
    #[cfg(windows)]
    fn supervised_process_ids_reports_assigned_children() {
        use std::process::Stdio;
        let _guard = job_test_lock();
        let mut child = std::process::Command::new("ping")
            .args(["-n", "60", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ping");
        let pid = child.id();
        super::assign_child(pid).expect("assign job");
        let ids = super::supervised_process_ids();
        assert!(
            ids.contains(&pid),
            "job membership must include the assigned child {pid}: {ids:?}"
        );
        // The list is the deduped union across retained jobs.
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids, sorted, "membership is sorted + deduped");
        let _ = child.kill();
        let _ = child.wait();
    }
}
