//! One-shot, resource-limited Boa subprocess. No Tauri startup or host APIs.
use crate::override_::{
    JsWorkerReply, JsWorkerRequest, JS_WORKER_MAGIC, MAX_JS_REQUEST_BYTES, MAX_JS_RESULT_BYTES,
};
use std::io::{Read, Write};

pub const WORKER_FLAG: &str = "--mfork-js-worker";
const MEMORY_LIMIT: usize = 256 * 1024 * 1024;

#[cfg(windows)]
struct Limits(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Limits {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn restrict() -> Result<Limits, String> {
    use windows_sys::Win32::System::{JobObjects::*, Threading::GetCurrentProcess};
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let guard = Limits(job);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
            | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_PROCESS_TIME;
        info.BasicLimitInformation.ActiveProcessLimit = 1;
        info.BasicLimitInformation.PerProcessUserTimeLimit = 10 * 10_000_000;
        info.ProcessMemoryLimit = MEMORY_LIMIT;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of_val(&info) as u32,
        ) == 0
            || AssignProcessToJobObject(job, GetCurrentProcess()) == 0
        {
            return Err(format!(
                "无法建立 JS Job 限制: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(guard)
    }
}

#[cfg(unix)]
struct Limits;
#[cfg(unix)]
fn restrict() -> Result<Limits, String> {
    unsafe {
        let memory = libc::rlimit {
            rlim_cur: MEMORY_LIMIT as _,
            rlim_max: MEMORY_LIMIT as _,
        };
        let cpu = libc::rlimit {
            rlim_cur: 10,
            rlim_max: 10,
        };
        let core = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::setrlimit(libc::RLIMIT_AS, &memory) != 0
            || libc::setrlimit(libc::RLIMIT_CPU, &cpu) != 0
            || libc::setrlimit(libc::RLIMIT_CORE, &core) != 0
        {
            return Err(format!(
                "无法建立 JS 资源限制: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(Limits)
}

fn read_request(mut input: impl Read) -> Result<JsWorkerRequest, String> {
    let mut header = [0; 4];
    input.read_exact(&mut header).map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes(header) as usize;
    if len > MAX_JS_REQUEST_BYTES {
        return Err("JS 请求超过大小限制".into());
    }
    let mut payload = vec![0; len];
    input.read_exact(&mut payload).map_err(|e| e.to_string())?;
    let request: JsWorkerRequest = serde_json::from_slice(&payload).map_err(|e| e.to_string())?;
    if request.magic != JS_WORKER_MAGIC || !request.config.is_object() {
        return Err("JS 请求格式无效".into());
    }
    Ok(request)
}

fn execute(request: JsWorkerRequest) -> Result<serde_json::Value, String> {
    use boa_engine::{Context, Source};
    let mut context = Context::default();
    context
        .runtime_limits_mut()
        .set_loop_iteration_limit(1_000_000);
    context.runtime_limits_mut().set_recursion_limit(128);
    context.runtime_limits_mut().set_stack_size_limit(512);
    context
        .eval(Source::from_bytes(&request.script))
        .map_err(|e| e.to_string())?;
    let config = serde_json::to_string(&request.config).map_err(|e| e.to_string())?;
    let quoted = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    let code = format!("JSON.stringify(main(JSON.parse({quoted})))");
    let result = context
        .eval(Source::from_bytes(&code))
        .map_err(|e| e.to_string())?;
    let string = result
        .as_string()
        .ok_or("main(config) 必须返回可序列化的对象")?;
    let text = string.to_std_string_escaped();
    if text.len() > MAX_JS_RESULT_BYTES {
        return Err("JS 结果超过大小限制".into());
    }
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if !value.is_object() {
        return Err("main(config) 必须返回对象".into());
    }
    Ok(value)
}

fn write_reply(mut output: impl Write, reply: JsWorkerReply) -> Result<(), String> {
    let mut data = serde_json::to_vec(&reply).map_err(|e| e.to_string())?;
    if data.len() > MAX_JS_RESULT_BYTES {
        data = serde_json::to_vec(&JsWorkerReply::Err("JS 结果超过大小限制".into())).unwrap();
    }
    output
        .write_all(&(data.len() as u32).to_le_bytes())
        .and_then(|_| output.write_all(&data))
        .and_then(|_| output.flush())
        .map_err(|e| e.to_string())
}

/// Limits are installed before reading even the request length. Failure is closed.
pub fn run() -> i32 {
    let limits = match restrict() {
        Ok(limits) => limits,
        Err(e) => {
            let _ = write_reply(std::io::stdout().lock(), JsWorkerReply::Err(e));
            return 2;
        }
    };
    let result = read_request(std::io::stdin().lock()).and_then(execute);
    let reply = match result {
        Ok(v) => JsWorkerReply::Ok(v),
        Err(e) => JsWorkerReply::Err(e),
    };
    let code = if write_reply(std::io::stdout().lock(), reply).is_ok() {
        0
    } else {
        2
    };
    // The Windows job kills its members when closed, so keep it through process exit.
    std::mem::forget(limits);
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_oversize_and_invalid_frames() {
        assert!(
            read_request(((MAX_JS_REQUEST_BYTES + 1) as u32).to_le_bytes().as_slice()).is_err()
        );
        assert!(read_request([0u8; 4].as_slice()).is_err());
    }
    #[test]
    fn safe_transform_and_loop_budget() {
        let request = |script: &str| JsWorkerRequest {
            magic: JS_WORKER_MAGIC,
            script: script.into(),
            config: serde_json::json!({"mode":"global"}),
        };
        assert_eq!(
            execute(request("function main(c) { c.ok=true; return c; }")).unwrap()["ok"],
            true
        );
        assert!(execute(request("function main(c) { while (true) {} }")).is_err());
        assert!(execute(request("function main(c) { return null; }")).is_err());
    }
}
