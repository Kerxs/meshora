//! 编进安装程序的载荷：要装的那些文件。格式见 build.rs。

use std::io::Read as _;

/// 一个要装的文件。
pub struct File {
    /// 文件名（不带路径，装到安装目录下）。
    pub name: String,
    /// 内容。
    pub data: Vec<u8>,
}

/// 编进来的载荷（压缩过的）。
const PACKED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/payload.bin"));

/// 解开编进来的载荷。开发时编的是空载荷，这里就是空的。
pub fn files() -> Result<Vec<File>, String> {
    unpack(PACKED)
}

fn unpack(packed: &[u8]) -> Result<Vec<File>, String> {
    let mut raw = Vec::new();
    flate2::read::DeflateDecoder::new(packed)
        .read_to_end(&mut raw)
        .map_err(|err| format!("安装内容解不开：{err}"))?;
    let broken = || "安装内容坏了：重新下载一次安装程序".to_string();
    let mut rest = raw.strip_prefix(b"MSHP1").ok_or_else(broken)?;
    let mut take = |n: usize| -> Result<&[u8], String> {
        if rest.len() < n {
            return Err(broken());
        }
        let (head, tail) = rest.split_at(n);
        rest = tail;
        Ok(head)
    };
    let count = u32::from_le_bytes(take(4)?.try_into().map_err(|_| broken())?);
    let mut files = Vec::new();
    for _ in 0..count {
        let len = u16::from_le_bytes(take(2)?.try_into().map_err(|_| broken())?) as usize;
        let name = String::from_utf8(take(len)?.to_vec()).map_err(|_| broken())?;
        // 只许是一个文件名：不许带路径，免得解到安装目录外面去
        if name.is_empty() || name.contains(['/', '\\', ':']) || name == "." || name == ".." {
            return Err(broken());
        }
        let size = u64::from_le_bytes(take(8)?.try_into().map_err(|_| broken())?);
        let data = take(usize::try_from(size).map_err(|_| broken())?)?.to_vec();
        files.push(File { name, data });
    }
    if !rest.is_empty() {
        return Err(broken());
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn pack(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut raw = b"MSHP1".to_vec();
        raw.extend_from_slice(&(files.len() as u32).to_le_bytes());
        for (name, data) in files {
            raw.extend_from_slice(&(name.len() as u16).to_le_bytes());
            raw.extend_from_slice(name.as_bytes());
            raw.extend_from_slice(&(data.len() as u64).to_le_bytes());
            raw.extend_from_slice(data);
        }
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&raw).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn round_trips() {
        let files = unpack(&pack(&[("meshora.exe", b"MZ..."), ("wintun.dll", b"dll")])).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].name, "meshora.exe");
        assert_eq!(files[0].data, b"MZ...");
        assert_eq!(files[1].name, "wintun.dll");
        assert!(unpack(&pack(&[])).unwrap().is_empty());
    }

    #[test]
    fn names_cannot_escape_the_install_folder() {
        for bad in ["../evil.dll", "sub\\x.dll", "C:x", ".."] {
            assert!(unpack(&pack(&[(bad, b"x")])).is_err(), "{bad}");
        }
    }

    #[test]
    fn truncated_or_garbage_payloads_are_refused() {
        let good = pack(&[("a", b"hello")]);
        assert!(unpack(&good[..good.len() / 2]).is_err());
        assert!(unpack(b"not deflate at all").is_err());
    }
}
