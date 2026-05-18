use std::{fmt::Debug, net::SocketAddr, path::Path};

use anyhow::Result;
use serde_json::Value;
use tracing::event;

use crate::{exec, model::Vector, shell_device};

const DEVICE_INFO_QUERY_SHELL: &str = "param get const.product.devicetype; param get const.product.model; param get const.product.name; echo '\t\t'; SP_daemon -deviceinfo";

#[derive(Debug, Clone)]
struct InnerDevice {
    tag: String,
    connect_type: String,
}

#[derive(Debug, Clone)]
pub struct Device {
    inner: InnerDevice,
    pub info: DeviceInfo,
}

#[derive(Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub main_screen: Vector,
    pub device_type: String,
    pub model: String,
    pub sn: String,
}

impl Debug for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("name", &self.name)
            .field("main_screen", &self.main_screen)
            .field("device_type", &self.device_type)
            .field("model", &self.model)
            .finish()
    }
}

impl DeviceInfo {
    async fn new(inner: &InnerDevice) -> anyhow::Result<Self> {
        let res = shell_device!(&inner.tag, DEVICE_INFO_QUERY_SHELL)
            .await?
            .to_string();
        let (param_str, deviceinfo_str) = res.split_once("\t\t").unwrap();
        let params: Vec<&str> = param_str.split("\n").map(|v| v.trim()).collect();
        let infos = deviceinfo_str
            .split("\n")
            .filter_map(|v| v.split_once(": "))
            .collect::<Vec<(&str, &str)>>();
        let sn = infos
            .iter()
            .find(|(k, _)| k.to_lowercase() == "sn")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        let main_screen = infos
            .iter()
            .find(|(k, _)| k.to_lowercase() == "activemode")
            .map(|(_, v)| {
                let (x, y) = v.split_once("x").unwrap();
                Vector::new(x.parse().unwrap(), y.parse().unwrap())
            })
            .unwrap_or_default();
        Ok(Self {
            name: params[2].to_string(),
            main_screen,
            device_type: params[0].to_string(),
            model: params[1].to_string(),
            sn,
        })
    }
}

impl Device {
    async fn new(inner: InnerDevice) -> anyhow::Result<Self> {
        let info = DeviceInfo::new(&inner).await?;
        Ok(Self { inner, info })
    }

    pub fn format_tag(&self) -> String {
        format!(
            "{} ({} {} {} {})",
            self.info.name,
            self.info.model,
            self.info.device_type,
            self.inner.connect_type,
            &self.secret_tag()
        )
    }

    fn secret_tag(&self) -> String {
        match &self.inner.tag.parse::<SocketAddr>() {
            Ok(_) => self.inner.tag.to_owned(),
            Err(_) => {
                // starts 3 and ends with 2, mid use * to hide
                let mut tag = self.inner.tag.clone();
                tag.replace_range(3..tag.len() - 2, "*");
                tag
            }
        }
    }

    pub async fn dump_layout_to_text(&self) -> Result<String> {
        Ok(
            shell_device!(&self.inner.tag, "export DUMPLAYOUT_TMP=$(uitest dumpLayout | cut -d ':' -f2-); cat $DUMPLAYOUT_TMP; rm $DUMPLAYOUT_TMP")
                .await?
                .to_string(),
        )
    }

    pub async fn dump_layout_to_file(&self, file: impl AsRef<Path>) -> Result<()> {
        let res = self.dump_layout_to_text().await?;
        let pretty_json = match serde_json::from_str::<Value>(&res) {
            Ok(json) => serde_json::to_string_pretty(&json)?, // 格式化
            Err(_) => res,                                    // 若不是合法 JSON，原样保存
        };

        std::fs::write(file, pretty_json)?;
        Ok(())
    }
}

pub async fn get_devices() -> Result<Vec<Device>> {
    let res = exec!("list", "targets", "-v").await?.to_string();
    let res = res.trim().to_string().replace("  ", " ");
    let inner_devices: Vec<InnerDevice> = res
        .lines()
        .filter_map(|line| {
            let args = line.split_whitespace().collect::<Vec<&str>>();
            if args.len() < 3 {
                event!(tracing::Level::ERROR, "Invalid device line: {}", line);
                None
            } else {
                Some(InnerDevice {
                    tag: args[0].to_string(),
                    connect_type: args[1].to_string(),
                })
            }
        })
        .collect();

    let mut devices = vec![];
    for inner_device in inner_devices {
        devices.push(Device::new(inner_device).await?);
    }

    Ok(devices)
}
