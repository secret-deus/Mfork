//! Versioned mutually authenticated IPC. Shared verbatim by GUI and service.
//! No secret is transmitted; all frames and the entire exchange are bounded.
use hmac::{Hmac, Mac};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::Sha256;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub const MAX_FRAME: usize = 16 * 1024;
pub const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION: u32 = 2;
type Key = [u8; 32];

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    version: u32,
    nonce: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Challenge {
    nonce: String,
    proof: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Signed {
    body: String,
    proof: String,
}

pub fn random_hex() -> Result<String, String> {
    let mut bytes = [0; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(hex(&bytes))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
pub fn parse_key(text: &str) -> Result<Key, String> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid service key/nonce; reinstall the service".into());
    }
    let mut out = [0; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(out)
}
fn mac(key: &Key, label: &[u8], parts: &[&[u8]]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("fixed HMAC key size");
    mac.update(b"Mfork service IPC v2\0");
    mac.update(&(label.len() as u32).to_be_bytes());
    mac.update(label);
    for part in parts {
        mac.update(&(part.len() as u32).to_be_bytes());
        mac.update(part);
    }
    mac
}
fn proof(key: &Key, label: &[u8], parts: &[&[u8]]) -> String {
    hex(&mac(key, label, parts).finalize().into_bytes())
}
fn verify(key: &Key, label: &[u8], parts: &[&[u8]], signature: &str) -> Result<(), String> {
    let signature = parse_key(signature)?;
    mac(key, label, parts)
        .verify_slice(&signature)
        .map_err(|_| "service authentication failed; no legacy fallback".into())
}
fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| "service exchange deadline exceeded".into())
}
fn read_exact(
    stream: &mut TcpStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> Result<(), String> {
    while !bytes.is_empty() {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|e| e.to_string())?;
        let n = stream
            .read(bytes)
            .map_err(|e| format!("service read: {e}"))?;
        if n == 0 {
            return Err("service closed connection".into());
        }
        bytes = &mut bytes[n..];
    }
    remaining(deadline)?;
    Ok(())
}
fn write_all(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> Result<(), String> {
    while !bytes.is_empty() {
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(|e| e.to_string())?;
        let n = stream
            .write(bytes)
            .map_err(|e| format!("service write: {e}"))?;
        if n == 0 {
            return Err("service write returned zero".into());
        }
        bytes = &bytes[n..];
    }
    remaining(deadline)?;
    Ok(())
}
fn read_frame<T: DeserializeOwned>(stream: &mut TcpStream, deadline: Instant) -> Result<T, String> {
    let mut header = [0; 4];
    read_exact(stream, &mut header, deadline)?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err("service frame exceeds 16 KiB".into());
    }
    let mut data = vec![0; len];
    read_exact(stream, &mut data, deadline)?;
    serde_json::from_slice(&data).map_err(|e| format!("invalid service frame: {e}"))
}
fn write_frame<T: Serialize>(
    stream: &mut TcpStream,
    data: &T,
    deadline: Instant,
) -> Result<(), String> {
    let data = serde_json::to_vec(data).map_err(|e| e.to_string())?;
    if data.is_empty() || data.len() > MAX_FRAME {
        return Err("service frame exceeds 16 KiB".into());
    }
    write_all(stream, &(data.len() as u32).to_be_bytes(), deadline)?;
    write_all(stream, &data, deadline)
}

pub fn client_call<Q: Serialize, R: DeserializeOwned>(
    stream: &mut TcpStream,
    key: &Key,
    request: &Q,
) -> Result<R, String> {
    let deadline = Instant::now() + EXCHANGE_TIMEOUT;
    let client = random_hex()?;
    write_frame(
        stream,
        &Hello {
            version: VERSION,
            nonce: client.clone(),
        },
        deadline,
    )?;
    let challenge: Challenge = read_frame(stream, deadline)?;
    parse_key(&challenge.nonce)?;
    let nonces = [client.as_bytes(), challenge.nonce.as_bytes()];
    // Authenticate the server before disclosing the command/config path.
    verify(key, b"server", &nonces, &challenge.proof)?;
    let body = serde_json::to_string(request).map_err(|e| e.to_string())?;
    let signature = proof(key, b"request", &[nonces[0], nonces[1], body.as_bytes()]);
    write_frame(
        stream,
        &Signed {
            body: body.clone(),
            proof: signature,
        },
        deadline,
    )?;
    let response: Signed = read_frame(stream, deadline)?;
    verify(
        key,
        b"response",
        &[
            nonces[0],
            nonces[1],
            body.as_bytes(),
            response.body.as_bytes(),
        ],
        &response.proof,
    )?;
    serde_json::from_str(&response.body).map_err(|e| e.to_string())
}

pub fn server_call<Q: DeserializeOwned, R: Serialize>(
    stream: &mut TcpStream,
    key: &Key,
    handle: impl FnOnce(Q) -> R,
) -> Result<(), String> {
    let deadline = Instant::now() + EXCHANGE_TIMEOUT;
    let hello: Hello = read_frame(stream, deadline)?;
    if hello.version != VERSION {
        return Err("unsupported service protocol; reinstall required".into());
    }
    parse_key(&hello.nonce)?;
    let server = random_hex()?;
    let nonces = [hello.nonce.as_bytes(), server.as_bytes()];
    write_frame(
        stream,
        &Challenge {
            nonce: server.clone(),
            proof: proof(key, b"server", &nonces),
        },
        deadline,
    )?;
    let request: Signed = read_frame(stream, deadline)?;
    verify(
        key,
        b"request",
        &[nonces[0], nonces[1], request.body.as_bytes()],
        &request.proof,
    )?;
    let request_value = serde_json::from_str(&request.body).map_err(|e| e.to_string())?;
    remaining(deadline)?;
    let response = handle(request_value);
    let body = serde_json::to_string(&response).map_err(|e| e.to_string())?;
    let signature = proof(
        key,
        b"response",
        &[
            nonces[0],
            nonces[1],
            request.body.as_bytes(),
            body.as_bytes(),
        ],
    );
    write_frame(
        stream,
        &Signed {
            body,
            proof: signature,
        },
        deadline,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        (client, listener.accept().unwrap().0)
    }
    #[test]
    fn mutual_auth_round_trip() {
        let (mut c, mut s) = pair();
        let key = [7; 32];
        let task = std::thread::spawn(move || server_call(&mut s, &key, |n: u32| n + 1).unwrap());
        assert_eq!(client_call::<_, u32>(&mut c, &key, &41).unwrap(), 42);
        task.join().unwrap();
    }
    #[test]
    fn fake_server_gets_no_request_or_token() {
        let (mut c, mut s) = pair();
        let task = std::thread::spawn(move || {
            let d = Instant::now() + Duration::from_secs(1);
            let hello: Hello = read_frame(&mut s, d).unwrap();
            let nonce = "11".repeat(32);
            write_frame(
                &mut s,
                &Challenge {
                    proof: proof(
                        &[1; 32],
                        b"server",
                        &[hello.nonce.as_bytes(), nonce.as_bytes()],
                    ),
                    nonce,
                },
                d,
            )
            .unwrap();
            assert!(read_frame::<Signed>(&mut s, d).is_err());
        });
        assert!(client_call::<_, u32>(&mut c, &[7; 32], &41).is_err());
        drop(c);
        task.join().unwrap();
    }
    #[test]
    fn proof_is_bound_to_nonce_role_and_body() {
        let k = [9; 32];
        let p = proof(&k, b"request", &[b"c", b"s", b"start"]);
        assert!(verify(&k, b"request", &[b"c", b"s", b"start"], &p).is_ok());
        for (role, parts) in [
            (b"server".as_slice(), [b"c".as_slice(), b"s", b"start"]),
            (b"request", [b"c", b"x", b"start"]),
            (b"request", [b"c", b"s", b"stop"]),
        ] {
            assert!(verify(&k, role, &parts, &p).is_err());
        }
    }
    #[test]
    fn frame_limits_and_absolute_deadline() {
        let (mut c, mut s) = pair();
        c.write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
            .unwrap();
        assert!(read_frame::<Hello>(&mut s, Instant::now() + Duration::from_secs(1)).is_err());
        assert!(write_frame(
            &mut c,
            &"x".repeat(MAX_FRAME),
            Instant::now() + Duration::from_secs(1)
        )
        .is_err());
        let (mut c, mut s) = pair();
        c.write_all(&[0]).unwrap();
        let start = Instant::now();
        assert!(read_frame::<Hello>(&mut s, start + Duration::from_millis(80)).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn keys_require_exact_csprng_format() {
        assert!(parse_key(&random_hex().unwrap()).is_ok());
        for s in ["", "old-short-token", &"A".repeat(64), &"g".repeat(64)] {
            assert!(parse_key(s).is_err());
        }
    }
    #[test]
    fn server_rejects_forged_client_before_running_command() {
        let (mut c, mut s) = pair();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = called.clone();
        let task = std::thread::spawn(move || {
            assert!(server_call(&mut s, &[7; 32], |_: u32| {
                observed.store(true, std::sync::atomic::Ordering::SeqCst);
                1u32
            })
            .is_err());
        });
        let d = Instant::now() + Duration::from_secs(1);
        write_frame(
            &mut c,
            &Hello {
                version: VERSION,
                nonce: "11".repeat(32),
            },
            d,
        )
        .unwrap();
        let _: Challenge = read_frame(&mut c, d).unwrap();
        write_frame(
            &mut c,
            &Signed {
                body: "41".into(),
                proof: "00".repeat(32),
            },
            d,
        )
        .unwrap();
        task.join().unwrap();
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    }
    #[test]
    fn server_rejects_legacy_version() {
        let (mut c, mut s) = pair();
        let task = std::thread::spawn(move || {
            assert!(server_call(&mut s, &[7; 32], |_: u32| 1u32).is_err())
        });
        write_frame(
            &mut c,
            &Hello {
                version: 1,
                nonce: "11".repeat(32),
            },
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        task.join().unwrap();
    }
    #[test]
    fn response_cannot_be_rebound_to_another_request_or_reply() {
        let k = [7; 32];
        let p = proof(&k, b"response", &[b"client", b"server", b"status", b"ok"]);
        assert!(verify(
            &k,
            b"response",
            &[b"client", b"server", b"start", b"ok"],
            &p
        )
        .is_err());
        assert!(verify(
            &k,
            b"response",
            &[b"client", b"server", b"status", b"modified"],
            &p
        )
        .is_err());
    }
}
