use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    net::Ipv4Addr,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub service: String,
    pub host: String,
    pub addresses: Vec<Ipv4Addr>,
    pub port: u16,
    pub properties: BTreeMap<String, String>,
}

pub fn discover(root: &Path, seconds: u64) -> Result<(), Box<dyn std::error::Error>> {
    println!("正在发现 AirPlay 设备（{seconds} 秒）……");
    let daemon = ServiceDaemon::new()?;
    let receiver = daemon.browse("_airplay._tcp.local.")?;
    let raop_receiver = daemon.browse("_raop._tcp.local.")?;
    let monitor = daemon.monitor()?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut devices = BTreeMap::<String, Device>::new();
    let mut raop = Vec::new();
    while Instant::now() < deadline {
        while let Ok(ServiceEvent::ServiceResolved(service)) = raop_receiver.try_recv() {
            raop.push(service);
        }
        while let Ok(event) = monitor.try_recv() {
            if let mdns_sd::DaemonEvent::Error(error) = event {
                eprintln!("mDNS: {error}");
            }
        }
        if let Ok(ServiceEvent::ServiceResolved(service)) =
            receiver.recv_timeout(Duration::from_millis(250))
        {
            let mut addresses: Vec<_> = service.get_addresses_v4().into_iter().collect();
            addresses.sort();
            if addresses.is_empty() {
                continue;
            }
            let properties = service
                .get_properties()
                .iter()
                .map(|p| (p.key().to_owned(), p.val_str().to_owned()))
                .collect::<BTreeMap<_, _>>();
            let fullname = service.get_fullname().to_owned();
            let name = fullname
                .strip_suffix("._airplay._tcp.local.")
                .unwrap_or(&fullname)
                .to_owned();
            let device = Device {
                name,
                service: fullname.clone(),
                host: service.get_hostname().to_owned(),
                addresses,
                port: service.get_port(),
                properties,
            };
            if !devices.contains_key(&fullname) {
                println!(
                    "找到：{} | {:?}:{} | pw={} | model={} | osvers={}",
                    device.name,
                    device.addresses,
                    device.port,
                    device
                        .properties
                        .get("pw")
                        .map(String::as_str)
                        .unwrap_or("未公布"),
                    device
                        .properties
                        .get("model")
                        .map(String::as_str)
                        .unwrap_or("未公布"),
                    device
                        .properties
                        .get("osvers")
                        .map(String::as_str)
                        .unwrap_or("未公布")
                );
            }
            devices.insert(fullname, device);
        }
    }
    daemon.stop_browse("_airplay._tcp.local.")?;
    daemon.stop_browse("_raop._tcp.local.")?;
    if let Ok(done) = daemon.shutdown() {
        let _ = done.recv_timeout(Duration::from_secs(2));
    }
    let mut devices = devices.into_values().collect::<Vec<_>>();
    for device in &mut devices {
        for service in &raop {
            let addresses = service.get_addresses_v4();
            if device
                .addresses
                .iter()
                .any(|address| addresses.contains(address))
            {
                for property in service.get_properties().iter() {
                    let key = property.key().to_owned();
                    let value = property.val_str().to_owned();
                    if key == "pw" && value == "true" {
                        device.properties.insert(key, value);
                    } else {
                        device.properties.entry(key).or_insert(value);
                    }
                }
            }
        }
        println!(
            "最终设备：{} | {:?}:{} | pw={}",
            device.name,
            device.addresses,
            device.port,
            device
                .properties
                .get("pw")
                .map(String::as_str)
                .unwrap_or("未公布")
        );
    }
    fs::write(
        root.join("devices.json"),
        serde_json::to_string_pretty(&devices)?,
    )?;
    println!(
        "发现 {} 台；地址与能力已保存到 devices.json（不含密码）。",
        devices.len()
    );
    if devices.is_empty() {
        return Err("未发现设备；确认同一局域网、私有网络防火墙权限和 HomePod 电源".into());
    }
    Ok(())
}
