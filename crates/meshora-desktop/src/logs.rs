//! 把日志留在内存里给界面看。客户端没有控制台，出了问题用户能看到、能复制的只有这里。

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

/// 最多留多少行。
const CAPACITY: usize = 500;

/// 最近的日志。克隆出来的都指向同一份。
#[derive(Clone, Default)]
pub struct LogBuffer {
    lines: Arc<Mutex<VecDeque<String>>>,
    /// 每条日志再抄一份给它（安卓上抄进 logcat，`adb logcat` 看得到）
    mirror: Option<fn(&str)>,
}

impl LogBuffer {
    /// 每条日志除了留在内存里，再交给 `mirror` 一份。
    pub fn with_mirror(mirror: fn(&str)) -> Self {
        Self {
            mirror: Some(mirror),
            ..Self::default()
        }
    }

    /// 现在留着的所有行，从旧到新。
    pub fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    fn push(&self, text: &str) {
        if let Some(mirror) = self.mirror {
            mirror(text);
        }
        let mut lines = self
            .lines
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for line in text.lines().filter(|line| !line.is_empty()) {
            if lines.len() == CAPACITY {
                lines.pop_front();
            }
            lines.push_back(line.to_owned());
        }
    }
}

/// 一条日志的写入器：先攒着，丢掉的时候整条交给 [`LogBuffer`]。
pub struct EventWriter {
    buffer: LogBuffer,
    pending: Vec<u8>,
}

impl io::Write for EventWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for EventWriter {
    fn drop(&mut self) {
        self.buffer.push(&String::from_utf8_lossy(&self.pending));
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = EventWriter;

    fn make_writer(&'a self) -> Self::Writer {
        EventWriter {
            buffer: self.clone(),
            pending: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn keeps_the_latest_lines() {
        let buffer = LogBuffer::default();
        for i in 0..CAPACITY + 10 {
            let mut writer = buffer.make_writer();
            writeln!(writer, "line {i}").unwrap();
        }
        let lines = buffer.lines();
        assert_eq!(lines.len(), CAPACITY);
        assert_eq!(lines[0], "line 10");
        assert_eq!(lines.last().unwrap(), &format!("line {}", CAPACITY + 9));
    }

    #[test]
    fn an_event_written_in_pieces_is_one_line() {
        let buffer = LogBuffer::default();
        {
            let mut writer = buffer.make_writer();
            writer.write_all(b"INFO ").unwrap();
            writer.write_all(b"hello\n").unwrap();
        }
        assert_eq!(buffer.lines(), ["INFO hello"]);
    }
}
