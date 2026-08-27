use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};

use obfstr::obfstr;
use serde_json::{Map, Value};

/// Discord Rich Presence over the local IPC pipe - length-prefixed JSON frames.
pub struct Discord {
    client_id: String,
    pipe: Option<File>,
    nonce: u64,
}

impl Discord {
    pub fn new(client_id: String) -> Self {
        Discord { client_id, pipe: None, nonce: 0 }
    }

    pub fn connected(&self) -> bool {
        self.pipe.is_some()
    }

    pub fn disconnect(&mut self) {
        self.pipe = None;
    }

    pub fn connect(&mut self) -> io::Result<()> {
        let mut pipe = open_pipe()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "no pipe"))?;
        let mut handshake = Map::new();
        handshake.insert(obfstr!("v").to_string(), Value::from(1));
        handshake.insert(obfstr!("client_id").to_string(), Value::from(self.client_id.clone()));
        write_frame(&mut pipe, 0, Value::Object(handshake).to_string().as_bytes())?;
        let _ = read_frame(&mut pipe)?;
        self.pipe = Some(pipe);
        Ok(())
    }

    fn send_activity_value(&mut self, activity: Value) -> io::Result<()> {
        let pipe = self
            .pipe
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "not connected"))?;
        self.nonce = self.nonce.wrapping_add(1);

        let mut args = Map::new();
        args.insert(obfstr!("pid").to_string(), Value::from(std::process::id()));
        args.insert(obfstr!("activity").to_string(), activity);

        let mut payload = Map::new();
        payload.insert(obfstr!("cmd").to_string(), Value::from(obfstr!("SET_ACTIVITY").to_string()));
        payload.insert(obfstr!("args").to_string(), Value::Object(args));
        payload.insert(obfstr!("nonce").to_string(), Value::from(self.nonce.to_string()));

        write_frame(pipe, 1, Value::Object(payload).to_string().as_bytes())?;
        let _ = read_frame(pipe)?;
        Ok(())
    }

    pub fn set_activity(&mut self, activity: Value) -> io::Result<()> {
        self.send_activity_value(activity)
    }

    pub fn clear(&mut self) -> io::Result<()> {
        self.send_activity_value(Value::Null)
    }
}

fn open_pipe() -> Option<File> {
    for i in 0..10 {
        let path = format!("{}{}", obfstr!(r"\\.\pipe\discord-ipc-"), i);
        if let Ok(f) = OpenOptions::new().read(true).write(true).open(&path) {
            return Some(f);
        }
    }
    None
}

fn write_frame(file: &mut File, opcode: u32, payload: &[u8]) -> io::Result<()> {
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&opcode.to_le_bytes());
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    file.write_all(&buf)?;
    file.flush()
}

fn read_frame(file: &mut File) -> io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 8];
    file.read_exact(&mut header)?;
    let opcode = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let mut body = vec![0u8; len];
    file.read_exact(&mut body)?;
    Ok((opcode, body))
}
