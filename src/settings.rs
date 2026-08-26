use std::{
    env, fs,
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) system_audio: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { system_audio: true }
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

    fn load_from(path: &Path) -> io::Result<Self> {
        match File::open(path) {
            Ok(file) => serde_json::from_reader(file)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
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
        assert!(Settings::load_from(&path).unwrap().system_audio);
        Settings {
            system_audio: false,
        }
        .save_to(&path)
        .unwrap();
        assert!(!Settings::load_from(&path).unwrap().system_audio);

        let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
        fs::create_dir(&temporary).unwrap();
        assert!(Settings { system_audio: true }.save_to(&path).is_err());
        assert!(!Settings::load_from(&path).unwrap().system_audio);
        fs::remove_dir(&temporary).unwrap();
        fs::write(&path, b"{").unwrap();
        assert_eq!(
            Settings::load_from(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
