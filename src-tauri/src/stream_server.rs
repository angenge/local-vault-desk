use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

/// 本地/局域网安全流媒体微服务。
///
/// 功能要点：
/// - 绑定 127.0.0.1（纯本机）或 0.0.0.0（局域网共享），32 字节 Hex 会话 Token 鉴权；
/// - 完整 HTTP/1.1 Range（支持单区间/负区间/HEAD），Content-Range 总长按真实文件大小回填；
/// - 随机读取基于解密缓存（见 RcloneService::read_file_stream_range），seek 不再全量重解；
/// - 锁定/退出时物理停机销毁。
pub struct MediaStreamServer {
    bind_ip: IpAddr,
    port: u16,
    token: String,
    running: Arc<AtomicBool>,
}

/// 一次随机范围读取的结果：实际起始偏移（负区间解析后）、读取到的字节、文件总大小。
pub type StreamReadResult = (u64, Vec<u8>, u64);

impl MediaStreamServer {
    /// 启动流媒体服务。
    ///
    /// - `get_size_fn`：按远程路径获取明文（解密后）文件总大小，用于回填 Content-Length/Content-Range。
    /// - `read_range_fn`：按下述签名做解密后随机读取，返回 (实际起始偏移, 字节, 文件总大小)。
    pub fn start<F, G>(
        allow_lan: bool,
        get_size_fn: G,
        read_range_fn: F,
    ) -> Result<Self, String>
    where
        F: Fn(&str, u64, u64) -> Result<StreamReadResult, String> + Send + Sync + 'static,
        G: Fn(&str) -> Result<u64, String> + Send + Sync + 'static,
    {
        let bind_ip: IpAddr = if allow_lan {
            IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))
        } else {
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
        };

        let listener = TcpListener::bind(SocketAddr::new(bind_ip, 0))
            .map_err(|e| format!("绑定流媒体端口失败: {}", e))?;
        let addr = listener
            .local_addr()
            .map_err(|e| format!("获取本地端口失败: {}", e))?;
        let port = addr.port();

        // 高强度 32 字节 CSPRNG Hex 随机 Token
        let token = generate_random_token();
        let running = Arc::new(AtomicBool::new(true));

        let running_clone = running.clone();
        let token_clone = token.clone();
        let get_size = Arc::new(get_size_fn);
        let read_fn = Arc::new(read_range_fn);

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
                        let g_fn = get_size.clone();
                        thread::spawn(move || {
                            let _ = handle_client(stream, &tok, r_fn, g_fn);
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
            bind_ip,
            port,
            token,
            running,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn is_lan_enabled(&self) -> bool {
        self.bind_ip.is_unspecified()
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    /// 获取本机 Loopback 直链
    pub fn get_stream_url(&self, remote_path: &str) -> String {
        let clean = remote_path.trim_start_matches('/');
        let encoded: String = percent_encode_path(clean);
        format!(
            "http://127.0.0.1:{}/stream/{}?token={}",
            self.port, encoded, self.token
        )
    }

    /// 获取局域网 IP 直链（供手机/iPad 等设备在同 WiFi 下访问）
    pub fn get_lan_stream_url(&self, remote_path: &str) -> String {
        let clean = remote_path.trim_start_matches('/');
        let encoded: String = percent_encode_path(clean);
        let host_ip = get_local_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
        format!(
            "http://{}:{}/stream/{}?token={}",
            host_ip, self.port, encoded, self.token
        )
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], self.port)));
    }
}

impl Drop for MediaStreamServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 自动探测本机活跃局域网 IPv4：遍历本机所有网卡接口，
/// 跳过回环与链路本地地址，优先选择 IPv4 私网地址。
/// 不依赖任何外网可达性，国内网络同样准确。
pub fn get_local_lan_ip() -> Option<String> {
    use std::net::Ipv4Addr;
    let addrs = if_addrs::get_if_addrs().ok()?;
    let mut candidates: Vec<String> = addrs
        .into_iter()
        .filter_map(|a| match a.ip() {
            IpAddr::V4(v4) => Some(v4),
            IpAddr::V6(_) => None,
        })
        .filter(|ip: &Ipv4Addr| !ip.is_loopback() && !ip.is_link_local())
        .map(|ip| ip.to_string())
        .collect();
    // 优先返回私网地址（192.168.x / 10.x / 172.16~31.x），避免 VPN 虚拟网卡等干扰顺序
    candidates.sort_by_key(|ip| {
        let is_private = ip.starts_with("192.168.")
            || ip.starts_with("10.")
            || (ip.starts_with("172.") && {
                let oct: u16 = ip.split('.').nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                (16..=31).contains(&oct)
            });
        !is_private
    });
    candidates.into_iter().next()
}

/// 使用操作系统 CSPRNG 生成 32 字节 Hex 随机 Token
fn generate_random_token() -> String {
    let mut buf = [0u8; 32];
    // getrandom 失败（极罕见）时退化为时间戳+栈地址哈希，避免完全无法启动
    if getrandom::getrandom(&mut buf).is_err() {
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
        let ptr = &extra as *const _ as usize;
        h.update(ptr.to_ne_bytes());
        return hex::encode(h.finalize());
    }
    hex::encode(buf)
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

fn handle_client<F, G>(
    mut stream: TcpStream,
    valid_token: &str,
    read_fn: Arc<F>,
    get_size_fn: Arc<G>,
) -> std::io::Result<()>
where
    F: Fn(&str, u64, u64) -> Result<StreamReadResult, String> + Send + Sync + 'static,
    G: Fn(&str) -> Result<u64, String> + Send + Sync + 'static,
{
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(std::time::Duration::from_secs(10)))?;

    let mut header_buf = [0u8; 8192];
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

    let (url_path, query_str) = match full_path.find('?') {
        Some(idx) => (&full_path[..idx], &full_path[idx + 1..]),
        None => (full_path, ""),
    };

    // 强 Token 鉴权：局域网/本机请求均必须携带匹配的会话 Token
    let token_ok = query_str
        .split('&')
        .any(|p| p.strip_prefix("token=") == Some(valid_token));

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

    let mime_type = get_mime_by_filename(&file_path);

    // 获取真实文件总大小（解密后明文长度），用于 Content-Length / Content-Range 回填
    let total = match get_size_fn(&file_path) {
        Ok(total) => total,
        Err(e) => {
            let resp = format!(
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                e.len(),
                e
            );
            let _ = stream.write_all(resp.as_bytes());
            return Ok(());
        }
    };

    let mut range_header: Option<&str> = None;
    for line in lines {
        if line.to_lowercase().starts_with("range:") {
            range_header = Some(line[6..].trim());
            break;
        }
    }

    // 解析 Range：支持单区间、负区间后缀（bytes=-N）；多区间暂不支持，回退整体读取
    if let Some(range_str) = range_header {
        let parsed = parse_http_range(range_str);
        // 多区间（含逗号）或无法解析：忽略 Range 头，走整体读取
        if range_str.contains(',') {
            // 直接按整体读取处理
        } else if let Some((start, end_opt)) = parsed {
            let start = match start {
                RangeStart::Suffix(n) => {
                    if n == 0 || total == 0 {
                        // 无内容
                        let _ = stream.write_all(
                            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */0\r\nContent-Length: 0\r\n\r\n"
                                .as_bytes(),
                        );
                        return Ok(());
                    }
                    total.saturating_sub(n)
                }
                RangeStart::Offset(s) => s,
            };

            if start >= total && total > 0 {
                let resp = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\n\r\n",
                    total
                );
                let _ = stream.write_all(resp.as_bytes());
                return Ok(());
            }

            // 计算读取量：显式 end 优先，其次默认 4MB 分块；均以文件总长为上界
            let default_chunk = 4 * 1024 * 1024u64;
            let want_len = match end_opt {
                Some(end) => end.saturating_sub(start).saturating_add(1),
                None => default_chunk,
            };
            let want_len = want_len.min(total.saturating_sub(start)).max(1);

            match read_fn(&file_path, start, want_len) {
                Ok((actual_start, data, actual_total)) => {
                    if data.is_empty() {
                        let resp = format!(
                            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\n\r\n",
                            actual_total
                        );
                        let _ = stream.write_all(resp.as_bytes());
                        return Ok(());
                    }
                    let actual_len = data.len() as u64;
                    let actual_end = actual_start + actual_len - 1;
                    let resp_header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: {}\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
                        mime_type, actual_start, actual_end, actual_total, actual_len
                    );
                    let _ = stream.write_all(resp_header.as_bytes());
                    if method == "GET" {
                        let _ = stream.write_all(&data);
                    }
                    return Ok(());
                }
                Err(e) => {
                    let resp = format!(
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                        e.len(),
                        e
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    return Ok(());
                }
            }
        }
    }

    // 无 Range（或 Range 无法解析）：以 200 OK + 完整 Content-Length 流式下发整个文件。
    // 浏览器/播放器直接打开 URL 时首请求不带 Range 头，此时回 416 会被浏览器判为页面错误。
    if total > 0 {
        let resp_header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
            mime_type, total
        );
        let _ = stream.write_all(resp_header.as_bytes());
        if method == "GET" {
            // 分块顺序下发，全程不把整份明文加载进内存
            let mut offset = 0u64;
            const CHUNK: u64 = 1024 * 1024;
            while offset < total {
                let want = CHUNK.min(total - offset);
                match read_fn(&file_path, offset, want) {
                    Ok((_, data, _)) => {
                        if data.is_empty() {
                            break;
                        }
                        if stream.write_all(&data).is_err() {
                            break; // 对端提前断开（如浏览器停止加载）
                        }
                        offset += data.len() as u64;
                    }
                    Err(_) => break,
                }
            }
        }
    } else {
        let resp_header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: 0\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
            mime_type
        );
        let _ = stream.write_all(resp_header.as_bytes());
    }

    Ok(())
}

enum RangeStart {
    Offset(u64),
    Suffix(u64),
}

/// 解析 HTTP Range（单区间）：`bytes=0-99`、`bytes=100-`、`bytes=-500`
fn parse_http_range(range_header: &str) -> Option<(RangeStart, Option<u64>)> {
    let stripped = range_header.trim().strip_prefix("bytes=")?;
    let parts: Vec<&str> = stripped.split('-').collect();
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    let first = parts[0].trim();
    let second = parts.get(1).map(|s| s.trim()).unwrap_or("");

    if first.is_empty() {
        // 后缀区间 bytes=-N
        let n: u64 = second.parse().ok()?;
        if n == 0 {
            return None;
        }
        Some((RangeStart::Suffix(n), None))
    } else {
        let start: u64 = first.parse().ok()?;
        let end: Option<u64> = if second.is_empty() {
            None
        } else {
            match second.parse::<u64>() {
                Ok(e) if e >= start => Some(e),
                _ => return None,
            }
        };
        Some((RangeStart::Offset(start), end))
    }
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