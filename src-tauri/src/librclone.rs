use serde_json::Value;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::sync::{Mutex, Once};

#[repr(C)]
struct RcloneRPCResult {
    output: *mut c_char,
    status: c_int,
}

extern "C" {
    fn RcloneInitialize();
    fn RcloneFinalize();
    fn RcloneRPC(method: *mut c_char, input: *mut c_char) -> RcloneRPCResult;
    fn RcloneFreeString(str: *mut c_char);
}

static INIT_ONCE: Once = Once::new();
static RPC_LOCK: Mutex<()> = Mutex::new(());

pub fn init() {
    INIT_ONCE.call_once(|| {
        // 调优 Go 垃圾回收阈值，避免 39,000+ 海量碎文件元数据创建销毁时高频 GC 引起的 CPU 尖刺
        if std::env::var("GOGC").is_err() {
            std::env::set_var("GOGC", "200");
        }
        unsafe {
            RcloneInitialize();
        }
    });
}

pub fn finalize() {
    let _lock = RPC_LOCK.lock().unwrap();
    unsafe {
        RcloneFinalize();
    }
}

pub fn rclone_rpc(method: &str, params: Value) -> Result<Value, String> {
    init();

    let method_c = CString::new(method).map_err(|e| format!("无效的方法名: {}", e))?;
    let input_str = params.to_string();
    let input_c = CString::new(input_str).map_err(|e| format!("无效的 JSON 字符串: {}", e))?;

    // 串行化 FFI C-ABI 跨语言调用，防止多线程并发 RPC 导致的底层数据竞争或调度卡顿
    let _lock = RPC_LOCK.lock().unwrap();

    let res = unsafe {
        RcloneRPC(
            method_c.as_ptr() as *mut c_char,
            input_c.as_ptr() as *mut c_char,
        )
    };

    if res.output.is_null() {
        return Err(format!("RcloneRPC 调用失败，返回空指针 (状态码: {})", res.status));
    }

    let output_str = unsafe {
        let s = CStr::from_ptr(res.output).to_string_lossy().into_owned();
        RcloneFreeString(res.output);
        s
    };

    if res.status != 200 {
        return Err(format!("Rclone RPC 错误 ({}): {}", res.status, output_str));
    }

    serde_json::from_str(&output_str).map_err(|e| format!("解析 Rclone RPC 响应失败: {} (内容: {})", e, output_str))
}
