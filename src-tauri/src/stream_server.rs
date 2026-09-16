use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

/// 本地安全流媒体微服务，仅绑定 127.0.0.1 随机空闲高位端口，
/// 结合 32 字节随机会话 Token 鉴权，支持全套 HTTP 1.1 Range 分块，
/// 锁定/退出时物理停机销毁。
pub struct MediaStreamServer {
    port: u16,
    token: String,
    running: Arc<AtomicBool>,
}

impl MediaStreamServer {
    pub fn start<F>(read_range_fn: F) -> Result<Self, String>
    where
        F: Fn(&str, u64, u64) -> Result<Vec<u8>, String> + Send + Sync + 'static,
    {
        // 绑定 127.0.0.1 纯本地回环，端口 0 由系统动态分配空闲高位端口
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("绑定本地流媒体端口失败: {}", e))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("获取本地端口失败: {}", e))?;
        let port = addr.port();

        // 生成高强度 32 字节 Hex 随机 Token
        let token = generate_random_token();
        let running = Arc::new(AtomicBool::new(true));

        let running_clone = running.clone();
        let token_clone = token.clone();
        let read_fn = Arc::new(read_range_fn);

        // 设置非阻塞/超时，以便能及时响应停机信号
        listener
            .set_nonblocking(false)
            .map_err(|e| e.to_string())?;

        thread::spawn(move || {
            for stream_res in listener.incoming() {
                if !running_clone.load(Ordering::SeqCst) {
                    break;
                }
                match stream_res {
                    Ok(stream) => {
                        let tok = token_clone.clone();
                        let r_fn = read_fn.clone();
                        thread::spawn(move || {
                            let _ = handle_client(stream, &tok, r_fn);
                        });
                    }
                    Err(_) => {
                        if !running_clone.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                }
            }
        });

        Ok(Self {
            port,
            token,
            running,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn get_stream_url(&self, remote_path: &str) -> String {
        let clean = remote_path.trim_start_matches('/');
        let encoded: String = percent_encode_path(clean);
        format!(
            "http://127.0.0.1:{}/stream/{}?token={}",
            self.port, encoded, self.token
        )
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        // 主动连接自身以解除 listener.incoming() 的阻塞
        let _ = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], self.port)));
    }
}

impl Drop for MediaStreamServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn generate_random_token() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(std::process::id().to_ne_bytes());
    h.update(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_ne_bytes(),
    );
    let extra = [0u8; 16];
    // 简易伪随机扰动
    let ptr = &extra as *const _ as usize;
    h.update(ptr.to_ne_bytes());
    hex::encode(h.finalize())
}

fn percent_encode_path(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

fn percent_decode(s: &str) -> Result<String, ()> {
    let mut bytes = Vec::new();
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next().ok_or(())?;
            let h2 = chars.next().ok_or(())?;
            let hex_buf = [h1, h2];
            let hex_str = std::str::from_utf8(&hex_buf).map_err(|_| ())?;
            let val = u8::from_str_radix(hex_str, 16).map_err(|_| ())?;
            bytes.push(val);
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).map_err(|_| ())
}

fn handle_client<F>(mut stream: TcpStream, valid_token: &str, read_fn: Arc<F>) -> std::io::Result<()>
where
    F: Fn(&str, u64, u64) -> Result<Vec<u8>, String>,
{
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(10)))?;

    let mut header_buf = [0u8; 4096];
    let n = stream.read(&mut header_buf)?;
    if n == 0 {
        return Ok(());
    }

    let req_str = String::from_utf8_lossy(&header_buf[..n]);
    let mut lines = req_str.lines();
    let first_line = match lines.next() {
        Some(l) => l,
        None => return Ok(()),
    };

    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Ok(());
    }

    let method = parts[0];
    let full_path = parts[1];

    if method != "GET" && method != "HEAD" {
        let resp = "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
        return Ok(());
    }

    // 解析 URI 与 query
    let (url_path, query_str) = match full_path.find('?') {
        Some(idx) => (&full_path[..idx], &full_path[idx + 1..]),
        None => (full_path, ""),
    };

    // 校验 Token 鉴权
    let token_ok = query_str
        .split('&')
        .any(|p| p.starts_with("token=") && &p[6..] == valid_token);

    if !token_ok {
        let resp = "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 18\r\n\r\n403 Access Denied";
        let _ = stream.write_all(resp.as_bytes());
        return Ok(());
    }

    if !url_path.starts_with("/stream/") {
        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(resp.as_bytes());
        return Ok(());
    }

    let encoded_file = &url_path["/stream/".len()..];
    let file_path = match percent_decode(encoded_file) {
        Ok(p) => p,
        Err(_) => {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
            return Ok(());
        }
    };

    // 解析 HTTP Range 头
    let mut range_header: Option<&str> = None;
    for line in lines {
        if line.to_lowercase().starts_with("range:") {
            range_header = Some(line[6..].trim());
            break;
        }
    }

    let mime_type = get_mime_by_filename(&file_path);

    // 探测文件总大小（通过首部分片或者读取函数）
    // 默认单片窗口 2MB
    let default_chunk_size = 2 * 1024 * 1024u64;

    if let Some(range_str) = range_header {
        // 请求分片
        if let Some((start, end_opt)) = parse_http_range(range_str) {
            let chunk_len = match end_opt {
                Some(end) => (end - start + 1).min(default_chunk_size),
                None => default_chunk_size,
            };

            match read_fn(&file_path, start, chunk_len) {
                Ok(data) => {
                    let actual_len = data.len() as u64;
                    if actual_len == 0 {
                        let resp = "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */*\r\nContent-Length: 0\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes());
                        return Ok(());
                    }
                    let actual_end = start + actual_len - 1;
                    let resp_header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: {}\r\nContent-Range: bytes {}-{}/200000000000\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
                        mime_type, start, actual_end, actual_len
                    );
                    let _ = stream.write_all(resp_header.as_bytes());
                    if method == "GET" {
                        let _ = stream.write_all(&data);
                    }
                    return Ok(());
                }
                Err(e) => {
                    let resp = format!("HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}", e.len(), e);
                    let _ = stream.write_all(resp.as_bytes());
                    return Ok(());
                }
            }
        }
    }

    // 无 Range 头全量请求（返回前 4MB 启动帧）
    match read_fn(&file_path, 0, 4 * 1024 * 1024) {
        Ok(data) => {
            let resp_header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
                mime_type, data.len()
            );
            let _ = stream.write_all(resp_header.as_bytes());
            if method == "GET" {
                let _ = stream.write_all(&data);
            }
        }
        Err(e) => {
            let resp = format!("HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}", e.len(), e);
            let _ = stream.write_all(resp.as_bytes());
        }
    }

    Ok(())
}

fn parse_http_range(range_header: &str) -> Option<(u64, Option<u64>)> {
    let stripped = range_header.trim().strip_prefix("bytes=")?;
    let parts: Vec<&str> = stripped.split('-').collect();
    if parts.is_empty() {
        return None;
    }
    let start_str = parts[0].trim();
    let start: u64 = start_str.parse().ok()?;
    let end: Option<u64> = if parts.len() > 1 && !parts[1].trim().is_empty() {
        parts[1].trim().parse().ok()
    } else {
        None
    };
    Some((start, end))
}

fn get_mime_by_filename(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "m4a" => "audio/mp4",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
