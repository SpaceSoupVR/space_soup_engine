use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Pose {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
}

impl Pose {
    pub fn new(position: Vec3, rotation: Quat) -> Self {
        Self {
            position: position.into(),
            rotation: [rotation.x, rotation.y, rotation.z, rotation.w],
        }
    }
    pub fn position(&self) -> Vec3 {
        Vec3::from(self.position)
    }
    pub fn rotation(&self) -> Quat {
        let r = self.rotation;
        let q = Quat::from_xyzw(r[0], r[1], r[2], r[3]);
        if q.length_squared() < 1e-6 {
            Quat::IDENTITY
        } else {
            q.normalize()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JointSample {
    pub name: String,
    pub pose: Pose,
    pub valid: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandSample {
    pub tracking_active: bool,
    pub grip: Option<Pose>,
    pub aim: Option<Pose>,
    pub joints: Vec<JointSample>,

    pub trigger: f32,
    pub squeeze: f32,
    pub stick: [f32; 2],
    pub stick_click: bool,
    pub btn_a: bool,
    pub btn_b: bool,
    pub btn_x: bool,
    pub btn_y: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocomotionSample {
    pub mode: String,
    pub player_offset: [f32; 3],
    pub player_yaw_deg: f32,
    pub teleport_aiming: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SceneSample {
    pub scene_name: String,
    pub object_count: usize,
    pub render_cuboids: usize,
    pub render_meshes: usize,
    pub active_animations: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct TimingSample {
    pub dt_seconds: f32,
    pub fps: f32,
    pub frame_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DebugPacket {
    pub head: Pose,
    pub left_hand: HandSample,
    pub right_hand: HandSample,
    pub locomotion: LocomotionSample,
    pub scene: SceneSample,
    pub timing: TimingSample,
    pub log_lines: Vec<String>,
}

pub mod sender {
    use super::DebugPacket;
    use std::io::Write;
    use std::net::TcpStream;

    pub fn send(stream: &mut TcpStream, packet: &DebugPacket) -> std::io::Result<()> {
        let mut line = serde_json::to_string(packet)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        stream.write_all(line.as_bytes())
    }
}

pub mod receiver {
    use super::DebugPacket;
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};

    pub fn listen(addr: &str) -> std::io::Result<TcpListener> {
        TcpListener::bind(addr)
    }

    pub struct PacketReader {
        reader: BufReader<TcpStream>,
    }

    impl PacketReader {
        pub fn new(stream: TcpStream) -> Self {
            Self {
                reader: BufReader::new(stream),
            }
        }

        pub fn read_packet(&mut self) -> Option<DebugPacket> {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).ok()?;
            if n == 0 {
                return None;
            }
            serde_json::from_str(line.trim()).ok()
        }
    }
}
