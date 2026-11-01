use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct VideoPlayer {
    child: Child,
    socket: PathBuf,
    paused: bool,
    muted: bool,
    volume: u8,
}

impl VideoPlayer {
    pub fn open(path: &Path) -> io::Result<Self> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let socket =
            std::env::temp_dir().join(format!("spellbook-mpv-{}-{nonce}.sock", std::process::id()));
        let child = Command::new("mpv")
            .args([
                "--no-config",
                "--no-terminal",
                "--really-quiet",
                "--vo=null",
                "--pause=yes",
            ])
            .arg(format!("--input-ipc-server={}", socket.display()))
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut player = Self {
            child,
            socket,
            paused: true,
            muted: false,
            volume: 70,
        };
        if let Err(error) = player.wait_for_socket() {
            let _ = player.child.kill();
            let _ = player.child.wait();
            let _ = std::fs::remove_file(&player.socket);
            return Err(error);
        }
        player.set_property("volume", "70")?;
        Ok(player)
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn volume(&self) -> u8 {
        self.volume
    }

    pub fn toggle_pause(&mut self) -> io::Result<()> {
        let paused = !self.paused;
        self.set_property("pause", if paused { "true" } else { "false" })?;
        self.paused = paused;
        Ok(())
    }

    pub fn toggle_mute(&mut self) -> io::Result<()> {
        let muted = !self.muted;
        self.set_property("mute", if muted { "true" } else { "false" })?;
        self.muted = muted;
        Ok(())
    }

    pub fn adjust_volume(&mut self, delta: i16) -> io::Result<()> {
        let volume = (i16::from(self.volume) + delta).clamp(0, 100) as u8;
        self.set_property("volume", &volume.to_string())?;
        self.volume = volume;
        Ok(())
    }

    pub fn position(&mut self) -> io::Result<f64> {
        let mut stream = self.connect()?;
        stream.write_all(b"{\"command\":[\"get_property\",\"time-pos\"],\"request_id\":1}\n")?;
        let response = read_response(&mut stream)?;
        let value = response
            .split("\"data\":")
            .nth(1)
            .and_then(|tail| tail.split([',', '}']).next())
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .ok_or_else(|| io::Error::other("mpv returned no playback position"))?;
        Ok(value)
    }

    fn set_property(&self, property: &str, value: &str) -> io::Result<()> {
        let mut stream = self.connect()?;
        write!(
            stream,
            "{{\"command\":[\"set_property\",\"{property}\",{value}],\"request_id\":1}}\n"
        )?;
        let response = read_response(&mut stream)?;
        let succeeded = response
            .split("\"error\"")
            .nth(1)
            .and_then(|tail| tail.split(':').nth(1))
            .and_then(|tail| tail.split('"').nth(1))
            == Some("success");
        if succeeded {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "mpv rejected the {property} control: {response}"
            )))
        }
    }

    fn connect(&self) -> io::Result<UnixStream> {
        UnixStream::connect(&self.socket)
    }

    fn wait_for_socket(&mut self) -> io::Result<()> {
        for _ in 0..30 {
            if let Some(status) = self.child.try_wait()? {
                return Err(io::Error::other(format!(
                    "mpv exited before opening its control socket ({status})"
                )));
            }
            match self.connect() {
                Ok(_) => return Ok(()),
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        || error.kind() == io::ErrorKind::ConnectionRefused =>
                {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "mpv control socket did not become ready",
        ))
    }
}

fn read_response(stream: &mut UnixStream) -> io::Result<String> {
    stream.set_read_timeout(Some(Duration::from_millis(150)))?;
    let mut response = Vec::new();
    let mut byte = [0; 1];
    while response.len() < 1024 {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => response.push(byte[0]),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(error);
            }
            Err(error) => return Err(error),
        }
    }
    if response.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "mpv returned no control response",
        ));
    }
    Ok(String::from_utf8_lossy(&response).into_owned())
}

impl Drop for VideoPlayer {
    fn drop(&mut self) {
        let _ = self.set_property("pause", "true");
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("spellbook-video-{}-{nonce}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn controls_playback_through_mpv_ipc_when_tools_are_available() {
        if Command::new("mpv").arg("--version").output().is_err()
            || Command::new("ffmpeg").arg("-version").output().is_err()
        {
            return;
        }
        let fixture = Fixture::new();
        let path = fixture.0.join("test.avi");
        let output = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=64x64:r=10:d=5",
                "-c:v",
                "mpeg4",
                "-y",
            ])
            .arg(&path)
            .output()
            .expect("ffmpeg is available");
        assert!(
            output.status.success(),
            "could not generate test video: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let mut player = VideoPlayer::open(&path).expect("start mpv for test video");
        assert!(player.is_paused());
        assert_eq!(player.volume(), 70);
        player.adjust_volume(5).unwrap();
        assert_eq!(player.volume(), 75);
        player.toggle_mute().unwrap();
        assert!(player.is_muted());
        player.toggle_pause().unwrap();
        assert!(!player.is_paused());
        let mut advanced = false;
        for _ in 0..20 {
            thread::sleep(Duration::from_millis(50));
            if player.position().is_ok_and(|position| position > 0.0) {
                advanced = true;
                break;
            }
        }
        assert!(advanced, "mpv playback position did not advance");
        player.toggle_pause().unwrap();
        assert!(player.is_paused());
    }
}
