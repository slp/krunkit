// SPDX-License-Identifier: Apache-2.0

use std::{path::PathBuf, str::FromStr};

use anyhow::{anyhow, Context, Result};
use clap::Parser;

#[derive(Clone, Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long)]
    pub cpus: u8,

    #[arg(long)]
    pub memory: u32,

    #[arg(long)]
    pub bootloader: bootloader::Config,

    #[arg(long = "device")]
    pub devices: Vec<device::VirtioDeviceConfig>,

    #[arg(long = "restful-uri")]
    pub restful_uri: String,
}

pub fn args_parse(s: String, label: &str, sz: Option<usize>) -> Result<Vec<String>> {
    let list: Vec<String> = s.split(',').map(|s| s.to_string()).collect();

    if let Some(size) = sz {
        if list.len() != size {
            return Err(anyhow!(
                "expected --{} argument to have {} comma-separated sub-arguments, found {}",
                label,
                size,
                list.len()
            ));
        }
    }

    Ok(list)
}

pub fn val_parse(s: String, label: &str) -> Result<String> {
    let vals: Vec<&str> = s.split('=').collect();

    match vals.len() {
        1 => Ok(vals[0].to_string()),
        2 => {
            let label_found = vals[0];
            if label_found != label {
                return Err(anyhow!(format!(
                    "expected label {}, found {}",
                    label, label_found
                )));
            }

            Ok(vals[1].to_string())
        }
        _ => Err(anyhow!(format!("invalid argument format: {}", s.clone()))),
    }
}

mod bootloader {
    use super::*;

    #[derive(Clone, Debug)]
    pub struct Config {
        fw: BootloaderFw,
        vstore: PathBuf,
        action: Action,
    }

    impl FromStr for Config {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let args = args_parse(s.to_string(), "bootloader", Some(3))?;

            let fw = BootloaderFw::from_str(&args[0])?;
            let v = Vstore::from_str(&args[1])?;
            let action = Action::from_str(&args[2])?;

            Ok(Self {
                fw,
                vstore: v.0,
                action,
            })
        }
    }

    #[derive(Clone, Debug)]
    pub enum BootloaderFw {
        Efi,
    }

    impl FromStr for BootloaderFw {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let string = s.to_string().to_lowercase();

            match string.as_str() {
                "efi" => Ok(Self::Efi),
                _ => Err(anyhow!("invalid bootloader firmware option: {}", string)),
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct Vstore(PathBuf);

    impl FromStr for Vstore {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let value = val_parse(s.to_string(), "variable-store")?;

            Ok(Self(
                PathBuf::from_str(&value).context("variable-store argument not a valid path")?,
            ))
        }
    }

    #[derive(Clone, Debug)]
    pub enum Action {
        Create,
    }

    impl FromStr for Action {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let string = s.to_string().to_lowercase();

            match string.as_str() {
                "create" => Ok(Self::Create),
                _ => Err(anyhow!("invalid bootloader action: {}", string)),
            }
        }
    }
}

pub mod device {
    use super::*;

    use std::{
        ffi::{c_char, CString},
        os::unix::ffi::OsStrExt,
        path::Path,
    };

    extern "C" {
        fn krun_set_root_disk(ctx_id: u32, c_disk_path: *const c_char) -> i32;
        fn krun_add_vsock_port(ctx_id: u32, port: u32, c_filepath: *const c_char) -> i32;
        fn krun_add_virtiofs(ctx_id: u32, c_tag: *const c_char, c_path: *const c_char) -> i32;
    }

    pub trait KrunContextSet {
        unsafe fn krun_ctx_set(&self, id: u32) -> Result<(), anyhow::Error>;
    }

    #[derive(Clone, Debug)]
    pub enum VirtioDeviceConfig {
        Blk(BlkConfig),
        Rng,
        Serial(SerialConfig),
        Vsock(VsockConfig),
        Net(NetConfig),
        Fs(FsConfig),
    }

    impl FromStr for VirtioDeviceConfig {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let args = args_parse(s.to_string(), "virtio", None)?;

            if args.is_empty() {
                return Err(anyhow!("no virtio device config found"));
            }

            let rest = args[1..].join(",");

            match &args[0][..] {
                "virtio-blk" => Ok(Self::Blk(BlkConfig::from_str(&rest)?)),
                "virtio-rng" => Ok(Self::Rng),
                "virtio-serial" => Ok(Self::Serial(SerialConfig::from_str(&rest)?)),
                "virtio-vsock" => Ok(Self::Vsock(VsockConfig::from_str(&rest)?)),
                "virtio-net" => Ok(Self::Net(NetConfig::from_str(&rest)?)),
                "virtio-fs" => Ok(Self::Fs(FsConfig::from_str(&rest)?)),
                _ => Err(anyhow!(format!(
                    "invalid virtio device label specified: {}",
                    args[0]
                ))),
            }
        }
    }

    impl KrunContextSet for VirtioDeviceConfig {
        unsafe fn krun_ctx_set(&self, id: u32) -> Result<(), anyhow::Error> {
            match self {
                Self::Blk(blk) => blk.krun_ctx_set(id),
                Self::Rng => unimplemented!(),
                Self::Serial(_) => unimplemented!(),
                Self::Vsock(vsock) => vsock.krun_ctx_set(id),
                Self::Net(_) => unimplemented!(),
                Self::Fs(fs) => fs.krun_ctx_set(id),
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct BlkConfig {
        path: PathBuf,
    }

    impl FromStr for BlkConfig {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let args = args_parse(s.to_string(), "virtio-blk", Some(1))?;

            Ok(Self {
                path: PathBuf::from_str(&val_parse(args[0].clone(), "path")?)
                    .context("path argument not a valid path")?,
            })
        }
    }

    impl KrunContextSet for BlkConfig {
        unsafe fn krun_ctx_set(&self, id: u32) -> Result<(), anyhow::Error> {
            let path_cstr = path_to_cstring(&self.path)?.as_ptr() as *const c_char;

            if krun_set_root_disk(id, path_cstr) < 0 {
                return Err(anyhow!("unable to set virtio-blk root disk"));
            }

            Ok(())
        }
    }

    #[derive(Clone, Debug)]
    pub struct SerialConfig {
        log_file_path: PathBuf,
    }

    impl FromStr for SerialConfig {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let args = args_parse(s.to_string(), "virtio-serial", Some(1))?;

            Ok(Self {
                log_file_path: PathBuf::from_str(&val_parse(args[0].clone(), "logFilePath")?)
                    .context("logFilePath argument not a valid path")?,
            })
        }
    }

    #[derive(Clone, Debug)]
    pub struct VsockConfig {
        port: u32,
        socket_url: PathBuf,
        action: VsockAction,
    }

    impl FromStr for VsockConfig {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let args = args_parse(s.to_string(), "virtio-vsock", Some(3))?;

            let port = u32::from_str(&val_parse(args[0].clone(), "port")?)
                .context("port argument invalid")?;
            let socket_url = PathBuf::from_str(&val_parse(args[1].clone(), "socketURL")?)
                .context("socketURL argument not a valid path")?;
            let action = VsockAction::from_str(&args[2])?;

            Ok(Self {
                port,
                socket_url,
                action,
            })
        }
    }

    impl KrunContextSet for VsockConfig {
        unsafe fn krun_ctx_set(&self, id: u32) -> Result<(), anyhow::Error> {
            let path_cstr = path_to_cstring(&self.socket_url)?.as_ptr() as *const c_char;

            if krun_add_vsock_port(id, self.port, path_cstr) < 0 {
                return Err(anyhow!(format!(
                    "unable to add vsock port {} for path {}",
                    self.port,
                    &self.socket_url.display()
                )));
            }

            Ok(())
        }
    }

    #[derive(Clone, Debug)]
    pub enum VsockAction {
        Listen,
    }

    impl FromStr for VsockAction {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let s = s.to_string().to_lowercase();

            match &s[..] {
                "listen" => Ok(Self::Listen),
                _ => Err(anyhow!("invalid vsock action")),
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct NetConfig {
        unix_socket_path: PathBuf,
        mac_address: String,
    }

    impl FromStr for NetConfig {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let args = args_parse(s.to_string(), "virtio-net", Some(2))?;

            Ok(Self {
                unix_socket_path: PathBuf::from_str(&val_parse(args[0].clone(), "unixSocketPath")?)
                    .context("unixSocketPath argument not a valid path")?,
                mac_address: val_parse(args[1].clone(), "mac")?,
            })
        }
    }

    #[derive(Clone, Debug)]
    pub struct FsConfig {
        shared_dir: PathBuf,
        mount_tag: PathBuf,
    }

    impl FromStr for FsConfig {
        type Err = anyhow::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            let args = args_parse(s.to_string(), "virtio-fs", None)?;

            if args.len() < 2 {
                return Err(anyhow!(
                    "expected at least 2 arguments, found {}",
                    args.len()
                ));
            }

            let shared_dir = PathBuf::from_str(&val_parse(args[0].clone(), "sharedDir")?)
                .context("sharedDir argument not a valid path")?;
            let mount_tag = PathBuf::from_str(&val_parse(args[1].clone(), "mountTag")?)
                .context("mountTag argument not a valid path")?;

            Ok(Self {
                shared_dir,
                mount_tag,
            })
        }
    }

    impl KrunContextSet for FsConfig {
        unsafe fn krun_ctx_set(&self, id: u32) -> Result<(), anyhow::Error> {
            let shared_dir_cstr = path_to_cstring(&self.shared_dir)?.as_ptr() as *const c_char;
            let mount_tag_cstr = path_to_cstring(&self.mount_tag)?.as_ptr() as *const c_char;

            if krun_add_virtiofs(id, mount_tag_cstr, shared_dir_cstr) < 0 {
                return Err(anyhow!(format!(
                    "unable to add virtiofs shared directory {} with mount tag {}",
                    &self.shared_dir.display(),
                    &self.mount_tag.display()
                )));
            }

            Ok(())
        }
    }

    fn path_to_cstring(path: &Path) -> Result<CString, anyhow::Error> {
        let cstring = CString::new(path.as_os_str().as_bytes()).context(format!(
            "unable to convert path {} into NULL-terminated C string",
            path.display()
        ))?;

        Ok(cstring)
    }
}
