use std::{
    env, fs,
    fs::{File, OpenOptions},
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};

use axum::http::Uri;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) system_audio: bool,
    pub(crate) notifications: bool,
    pub(crate) listen_address: IpAddr,
    pub(crate) listen_port: u16,
    pub(crate) share_base_url: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            system_audio: true,
            notifications: true,
            listen_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
            listen_port: 8877,
            share_base_url: None,
        }
    }
}

impl Settings {
    pub(crate) fn load() -> io::Result<Self> {
        Self::load_from(&settings_path(
            env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            env::var_os("HOME").map(PathBuf::from),
        )?)
    }

    pub(crate) fn save(&self) -> io::Result<()> {
        self.save_to(&settings_path(
            env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            env::var_os("HOME").map(PathBuf::from),
        )?)
    }

    pub(crate) fn bind(&self) -> io::Result<SocketAddr> {
        validate_bind(SocketAddr::new(self.listen_address, self.listen_port))
    }

    pub(crate) fn with_network(
        &self,
        listen_address: &str,
        listen_port: &str,
        share_base_url: &str,
    ) -> io::Result<Self> {
        let listen_address = listen_address
            .trim()
            .parse()
            .map_err(|_| invalid("Listen address must be an IP address"))?;
        let listen_port = listen_port
            .trim()
            .parse()
            .map_err(|_| invalid("Port must be between 1 and 65535"))?;
        validate_bind(SocketAddr::new(listen_address, listen_port))?;
        let mut settings = self.clone();
        settings.listen_address = listen_address;
        settings.listen_port = listen_port;
        settings.share_base_url = parse_base_url(share_base_url)?;
        Ok(settings)
    }

    fn load_from(path: &Path) -> io::Result<Self> {
        match File::open(path) {
            Ok(file) => {
                let settings: Self = serde_json::from_reader(file)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                settings.validate()
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    fn save_to(&self, path: &Path) -> io::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("settings path has no parent"))?;
        fs::create_dir_all(parent)?;
        let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
        if let Err(error) = fs::remove_file(&temporary)
            && error.kind() != io::ErrorKind::NotFound
        {
            return Err(error);
        }
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            serde_json::to_writer(&mut file, self).map_err(io::Error::other)?;
            file.sync_all()?;
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    fn validate(self) -> io::Result<Self> {
        self.bind()?;
        if let Some(base_url) = &self.share_base_url
            && parse_base_url(base_url)?.as_deref() != Some(base_url)
        {
            return Err(invalid("Share base URL is not normalized"));
        }
        Ok(self)
    }
}

pub(crate) fn validate_bind(address: SocketAddr) -> io::Result<SocketAddr> {
    if address.port() == 0
        || address.ip().is_unspecified()
        || address.ip().is_multicast()
        || matches!(address.ip(), IpAddr::V4(ip) if ip.is_broadcast())
    {
        Err(invalid(
            "Listen address must be unicast and port must be between 1 and 65535",
        ))
    } else {
        Ok(address)
    }
}

fn parse_base_url(value: &str) -> io::Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let uri = value
        .parse::<Uri>()
        .map_err(|_| invalid("Share base URL must be http://host:port or https://host:port"))?;
    let scheme = uri.scheme_str();
    let authority = uri.authority();
    if !matches!(scheme, Some("http" | "https"))
        || authority.is_none_or(|authority| {
            authority.as_str().contains('@')
                || authority.host().is_empty()
                || authority.port_u16().is_none_or(|port| port == 0)
        })
        || uri.path() != "/"
        || uri.query().is_some()
    {
        return Err(invalid(
            "Share base URL must be http://host:port or https://host:port",
        ));
    }
    Ok(Some(format!(
        "{}://{}",
        scheme.expect("scheme checked above"),
        authority.expect("authority checked above")
    )))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn settings_path(xdg: Option<PathBuf>, home: Option<PathBuf>) -> io::Result<PathBuf> {
    let directory = xdg.filter(|path| path.is_absolute()).or_else(|| {
        home.filter(|path| path.is_absolute())
            .map(|path| path.join(".config"))
    });
    directory
        .map(|path| path.join("aercast/settings.json"))
        .ok_or_else(|| io::Error::other("no absolute XDG configuration directory is available"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_failed_replace_preserves_the_last_file() {
        assert_eq!(
            settings_path(Some("/xdg".into()), Some("/home/test".into())).unwrap(),
            Path::new("/xdg/aercast/settings.json")
        );
        assert_eq!(
            settings_path(Some("relative".into()), Some("/home/test".into())).unwrap(),
            Path::new("/home/test/.config/aercast/settings.json")
        );
        assert!(settings_path(None, None).is_err());

        let directory = env::temp_dir().join(format!("aercast-settings-{}", std::process::id()));
        let path = directory.join("settings.json");
        let _ = fs::remove_dir_all(&directory);
        let defaults = Settings::load_from(&path).unwrap();
        assert!(defaults.system_audio);
        assert!(defaults.notifications);
        assert_eq!(defaults.bind().unwrap(), "127.0.0.1:8877".parse().unwrap());
        Settings {
            system_audio: false,
            notifications: false,
            listen_address: "192.168.1.10".parse().unwrap(),
            listen_port: 9000,
            share_base_url: Some("https://share.example:443".to_owned()),
        }
        .save_to(&path)
        .unwrap();
        let saved = Settings::load_from(&path).unwrap();
        assert!(!saved.system_audio);
        assert!(!saved.notifications);
        assert_eq!(saved.bind().unwrap(), "192.168.1.10:9000".parse().unwrap());
        assert_eq!(
            saved.share_base_url.as_deref(),
            Some("https://share.example:443")
        );
        let before_failed_save = fs::read(&path).unwrap();

        let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::create_dir(&temporary).unwrap();
        assert!(Settings::default().save_to(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), before_failed_save);
        assert!(!Settings::load_from(&path).unwrap().system_audio);
        fs::remove_dir(&temporary).unwrap();
        fs::write(
            &path,
            br#"{"system_audio":false,"listen_address":"192.168.1.10","listen_port":9000,"share_base_url":null}"#,
        )
        .unwrap();
        assert!(Settings::load_from(&path).unwrap().notifications);
        fs::write(&path, b"{").unwrap();
        assert_eq!(
            Settings::load_from(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn network_inputs_are_validated_and_normalized() {
        let settings = Settings::default()
            .with_network(" 192.168.1.10 ", "9000", " https://share.example:443/ ")
            .unwrap();
        assert_eq!(
            settings.bind().unwrap(),
            "192.168.1.10:9000".parse().unwrap()
        );
        assert_eq!(
            settings.share_base_url.as_deref(),
            Some("https://share.example:443")
        );

        for (address, port, base_url) in [
            ("0.0.0.0", "8877", ""),
            ("224.0.0.1", "8877", ""),
            ("255.255.255.255", "8877", ""),
            ("127.0.0.1", "0", ""),
            ("localhost", "8877", ""),
            ("127.0.0.1", "8877", "ftp://share.example:21"),
            ("127.0.0.1", "8877", "http://share.example"),
            ("127.0.0.1", "8877", "http://user@share.example:80"),
            ("127.0.0.1", "8877", "http://share.example:80/path"),
        ] {
            assert!(
                Settings::default()
                    .with_network(address, port, base_url)
                    .is_err()
            );
        }
    }
}
