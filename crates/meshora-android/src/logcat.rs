//! 日志、panic 抄进 logcat：安卓上标准错误没人看，`adb logcat -s Meshora` 才看得到。

use std::ffi::{CString, c_char, c_int};

/// logcat 的级别：INFO
const INFO: c_int = 4;
/// logcat 的级别：ERROR
const ERROR: c_int = 6;

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

fn send(priority: c_int, text: &str) {
    let text = CString::new(text.replace('\0', " ")).unwrap_or_default();
    // SAFETY: 两个指针都指向以 0 结尾、活到调用结束的字符串；liblog 不留着它们
    #[allow(unsafe_code)]
    unsafe {
        __android_log_write(priority, c"Meshora".as_ptr(), text.as_ptr());
    }
}

/// 一条日志（可能多行）
pub(crate) fn write(text: &str) {
    for line in text.lines().filter(|line| !line.is_empty()) {
        send(INFO, line);
    }
}

/// panic 时把原因和位置也写进 logcat，再照常处理
pub(crate) fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        send(ERROR, &format!("panic：{info}"));
        default(info);
    }));
}
