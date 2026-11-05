//! Keep a Windows helper and its descendants in one kill-on-close job.

use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
    },
};

pub(super) struct Job(OwnedHandle);

impl Job {
    pub(super) fn new() -> io::Result<Self> {
        // No name or inheritable handle: only this generation owns the job.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let success = unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        if success == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    pub(super) fn assign_and_resume(&self, child: &tokio::process::Child) -> io::Result<()> {
        let handle = child.raw_handle().expect("new child has a process handle");
        let pid = child.id().expect("new child has a process ID");
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // CREATE_SUSPENDED prevents any helper code or descendants from running
        // before assignment. std closes the primary-thread handle at spawn, so
        // find that still-suspended process's sole initial thread via Toolhelp.
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut found = None;
        let mut present = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
        let mut count = 0;
        while present != 0 {
            if (entry.dwSize as usize)
                < std::mem::offset_of!(THREADENTRY32, th32OwnerProcessID)
                    + std::mem::size_of::<u32>()
            {
                return Err(io::Error::other("Windows thread entry is incomplete"));
            }
            count += 1;
            if count > 16_384 {
                return Err(io::Error::other("Windows thread inspection limit exceeded"));
            }
            if entry.th32OwnerProcessID == pid && found.replace(entry.th32ThreadID).is_some() {
                return Err(io::Error::other(
                    "suspended helper has multiple initial threads",
                ));
            }
            present = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
            return Err(error);
        }
        let thread_id =
            found.ok_or_else(|| io::Error::other("suspended helper thread was not found"))?;
        let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
        match unsafe { ResumeThread(thread.as_raw_handle()) } {
            1 => Ok(()),
            u32::MAX => Err(io::Error::last_os_error()),
            _ => Err(io::Error::other(
                "helper thread was not initially suspended",
            )),
        }
    }

    pub(super) fn terminate(&self) -> io::Result<()> {
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
