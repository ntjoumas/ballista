// Copyright (c) Kiran Ayyagari. All rights reserved.
// Copyright (c) Diridium Technologies Inc. All rights reserved.
// Licensed under the MPL-2.0 License. See LICENSE file in the project root.

use anyhow::Error;
use home::env::Env;
use home::env::OS_ENV;
use openssl::x509::store::{X509Store, X509StoreBuilder};
use openssl::x509::X509;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionEntry {
    pub address: String,
    #[serde(rename = "heapSize")]
    pub heap_size: String,
    pub icon: String,
    pub id: String,
    #[serde(rename = "javaHome")]
    pub java_home: String,
    #[serde(rename = "javaArgs")]
    pub java_args: Option<String>,
    pub name: String,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default = "get_verify")]
    pub verify: bool,
    #[serde(default = "get_default_group")]
    pub group: String,
    #[serde(default = "get_default_environment")]
    pub environment: String,
    #[serde(default = "get_default_notes")]
    pub notes: String,
    #[serde(default = "get_default_donotcache")]
    pub donotcache: bool,
    #[serde(default, rename = "lastConnected")]
    pub last_connected: Option<i64>,
    #[serde(default, rename = "groupOrder")]
    pub group_order: i64,
    #[serde(default, rename = "environmentOrder")]
    pub environment_order: i64,
    #[serde(default, rename = "sortOrder")]
    pub sort_order: i64,
    #[serde(default, rename = "showConsole")]
    pub show_console: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportedConnectionEntry {
    #[serde(flatten)]
    connection: ConnectionEntry,
    #[serde(default, rename = "iconDataUrl", alias = "exportedIconDataUrl")]
    icon_data_url: Option<String>,
}

pub struct ConnectionStore {
    con_cache: Mutex<HashMap<String, Arc<ConnectionEntry>>>,
    con_location: PathBuf,
    pub cache_dir: PathBuf,
    icons_dir: PathBuf,
    cert_store: Mutex<Arc<X509Store>>,
    trusted_certs_location: PathBuf,
}

impl Default for ConnectionEntry {
    fn default() -> Self {
        let empty_str = String::from("");
        ConnectionEntry {
            address: empty_str.clone(),
            heap_size: String::from("512m"),
            icon: empty_str.clone(),
            id: Uuid::new_v4().to_string(),
            java_home: find_java_home(),
            java_args: Option::from(empty_str.clone()),
            name: empty_str.clone(),
            username: None,
            password: None,
            verify: true,
            group: get_default_group(),
            environment: get_default_environment(),
            notes: get_default_notes(),
            donotcache: get_default_donotcache(),
            last_connected: None,
            group_order: 0,
            environment_order: 0,
            sort_order: 0,
            show_console: false,
        }
    }
}

impl ConnectionStore {
    pub fn init(data_dir_path: PathBuf) -> Result<Self, Error> {
        let con_location = data_dir_path.join("ballista-data.json");
        let mut con_location_file = File::open(&con_location);
        if let Err(_e) = con_location_file {
            con_location_file = File::create(&con_location);
        }
        let con_location_file = con_location_file?;

        let mut cache = HashMap::new();
        let data: serde_json::Result<HashMap<String, ConnectionEntry>> =
            serde_json::from_reader(con_location_file);
        match data {
            Ok(data) => {
                for (id, ce) in data {
                    cache.insert(id, Arc::new(ce));
                }
            }
            Err(e) => {
                println!("{}", e);
            }
        }

        let trusted_certs_location = data_dir_path.join("ballista-trusted-certs.json");
        let certs = parse_trusted_certs(&trusted_certs_location);
        let cert_store = create_cert_store(certs);

        let cache_dir = data_dir_path.join("cache");
        if !cache_dir.exists() {
            fs::create_dir(&cache_dir)?;
        }

        let icons_dir = data_dir_path.join("icons");
        if !icons_dir.exists() {
            fs::create_dir(&icons_dir)?;
        }

        Ok(ConnectionStore {
            con_location,
            con_cache: Mutex::new(cache),
            cert_store: Mutex::new(Arc::new(cert_store)),
            trusted_certs_location,
            cache_dir,
            icons_dir,
        })
    }

    pub fn to_json_array_string(&self) -> String {
        let cache = self.con_cache.lock().expect("connection cache lock poisoned");
        let mut sb = String::with_capacity(1024);
        let len = cache.len();
        sb.push('[');
        for (pos, ce) in cache.values().enumerate() {
            let c = serde_json::to_string(ce).unwrap_or_default();
            sb.push_str(c.as_str());
            if pos + 1 < len {
                sb.push(',');
            }
        }
        sb.push(']');

        sb
    }

    pub fn to_json_array_string_with_icons(&self) -> String {
        let cache = self.con_cache.lock().expect("connection cache lock poisoned");
        let values: Vec<Value> = cache
            .values()
            .map(|ce| self.connection_entry_json(ce.as_ref()))
            .collect();

        serde_json::to_string(&values).unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<Arc<ConnectionEntry>> {
        let cs = self.con_cache.lock().expect("connection cache lock poisoned");
        let val = cs.get(id);
        if let Some(val) = val {
            return Some(Arc::clone(val));
        }
        None
    }

    pub fn to_json_value(&self, id: &str) -> Option<Value> {
        let cs = self.con_cache.lock().expect("connection cache lock poisoned");
        cs.get(id).map(|ce| self.connection_entry_json(ce.as_ref()))
    }

    pub fn export(&self, file_path: &str, ids: Option<Vec<String>>) -> Result<String, Error> {
        let cache = self.con_cache.lock().expect("connection cache lock poisoned");
        let mut entries: Vec<Value> = match ids {
            Some(ids) => ids
                .into_iter()
                .filter_map(|id| cache.get(&id).map(|ce| self.exportable_connection_entry(ce.as_ref())))
                .collect(),
            None => cache
                .values()
                .map(|ce| self.exportable_connection_entry(ce.as_ref()))
                .collect(),
        };
        drop(cache);

        entries.sort_by(|a, b| {
            export_order_key(a).cmp(&export_order_key(b))
        });

        let payload = serde_json::to_string_pretty(&entries)?;
        fs::write(file_path, payload)?;

        Ok(serde_json::json!({
            "status": "ok",
            "total": entries.len(),
        })
        .to_string())
    }

    pub fn preview_import(&self, file_path: &str) -> Result<String, Error> {
        let entries = Self::read_import_entries(file_path)?
            .into_iter()
            .map(|entry| self.import_preview_entry(&entry))
            .collect::<Vec<_>>();
        Ok(serde_json::to_string(&entries)?)
    }

    pub fn save(&self, mut ce: ConnectionEntry) -> Result<String, Error> {
        let is_new = ce.id.is_empty();
        if is_new {
            ce.id = uuid::Uuid::new_v4().to_string();
        }

        let mut jh = ce.java_home.trim().to_string();
        if jh.is_empty() {
            jh = find_java_home();
        }
        ce.java_home = jh;

        if let Some(ref username) = ce.username {
            let username = username.trim();
            if username.is_empty() {
                ce.username = None;
            }
        }

        if let Some(ref password) = ce.password {
            let password = password.trim();
            if password.is_empty() {
                ce.password = None;
            }
        }

        self.prepare_managed_icon(&mut ce)?;

        let mut cache = self
            .con_cache
            .lock()
            .expect("connection cache lock poisoned");
        let is_existing = cache.contains_key(&ce.id);
        let group_name = ce.group.trim().to_string();
        let environment_name = ce.environment.trim().to_string();

        if (!is_existing || is_new) && ce.group_order == 0 {
            ce.group_order = cache
                .values()
                .find(|entry| entry.group.trim() == group_name)
                .map(|entry| entry.group_order)
                .unwrap_or_else(|| {
                    cache.values()
                        .map(|entry| entry.group_order)
                        .max()
                        .unwrap_or(-1)
                        + 1
                });
        }

        if (!is_existing || is_new) && ce.environment_order == 0 {
            ce.environment_order = cache
                .values()
                .find(|entry| {
                    entry.group.trim() == group_name && entry.environment.trim() == environment_name
                })
                .map(|entry| entry.environment_order)
                .unwrap_or_else(|| {
                    cache.values()
                        .filter(|entry| entry.group.trim() == group_name)
                        .map(|entry| entry.environment_order)
                        .max()
                        .unwrap_or(-1)
                        + 1
                });
        }

        if (!is_existing || is_new) && ce.sort_order == 0 {
            ce.sort_order = cache
                .values()
                .filter(|entry| {
                    entry.group.trim() == group_name && entry.environment.trim() == environment_name
                })
                .map(|entry| entry.sort_order)
                .max()
                .unwrap_or(-1)
                + 1;
        }

        let data = serde_json::to_string(&ce)?;
        cache.insert(ce.id.clone(), Arc::new(ce));
        drop(cache);
        self.write_connections_to_disk()?;
        Ok(data)
    }

    pub fn delete(&self, id: &str) -> Result<(), Error> {
        self.con_cache.lock().expect("connection cache lock poisoned").remove(id);
        self.remove_managed_icon(id)?;
        self.write_connections_to_disk()?;
        Ok(())
    }

    pub fn import(&self, file_path: &str, overwrite: bool) -> Result<String, Error> {
        let data = Self::read_import_entries(file_path)?;
        self.import_entries(data, overwrite)
    }

    pub fn import_entries(
        &self,
        data: Vec<ImportedConnectionEntry>,
        overwrite: bool,
    ) -> Result<String, Error> {
        let data = data
            .into_iter()
            .map(|mut entry| {
                let mut ce = entry.connection;
                if ce.id.trim().is_empty() {
                    ce.id = Uuid::new_v4().to_string();
                }
                entry.connection = ce;
                entry
            })
            .collect::<Vec<_>>();

        // Check for collisions with existing connections
        let cache = self.con_cache.lock().expect("connection cache lock poisoned");
        let duplicates: Vec<String> = data
            .iter()
            .filter(|entry| cache.contains_key(&entry.connection.id))
            .map(|entry| entry.connection.name.clone())
            .collect();
        drop(cache);

        if !duplicates.is_empty() && !overwrite {
            let result = serde_json::json!({
                "status": "duplicates",
                "names": duplicates,
                "total": data.len(),
            });
            return Ok(result.to_string());
        }

        let mut count = 0;
        let java_home = find_java_home();
        for mut entry in data {
            let mut ce = entry.connection;
            ce.java_home = java_home.clone();
            ce.username = ce
                .username
                .map(|username| username.trim().to_string())
                .filter(|username| !username.is_empty());
            ce.password = ce
                .password
                .map(|password| password.trim().to_string())
                .filter(|password| !password.is_empty());

            if let Some(data_url) = entry.icon_data_url.take() {
                ce.icon = self.write_managed_icon_data_url(&ce.id, &data_url)?;
            } else if Path::new(ce.icon.trim()).is_file() {
                self.prepare_managed_icon(&mut ce)?;
            } else {
                self.remove_managed_icon(&ce.id)?;
                ce.icon = String::new();
            }
            self.con_cache
                .lock()
                .expect("connection cache lock poisoned")
                .insert(ce.id.clone(), Arc::new(ce));
            count += 1;
        }

        self.write_connections_to_disk()?;
        let result = serde_json::json!({
            "status": "ok",
            "total": count,
        });
        Ok(result.to_string())
    }

    pub fn add_trusted_cert(&self, cert_der: &str) -> Result<(), Error> {
        let mut certs = parse_trusted_certs(&self.trusted_certs_location);
        let mut hasher = Sha256::new();
        hasher.update(cert_der);
        let hash = hasher.finalize();
        let hash = hex::encode(&hash);

        let cert_der = openssl::base64::decode_block(cert_der)?;
        let cert = X509::from_der(cert_der.as_slice())?;
        if let None = certs.get(&hash) {
            certs.insert(hash, cert);
        }

        let mut der_certs = FxHashMap::default();
        for (key, c) in &certs {
            let der = c.to_der()?;
            let der = openssl::base64::encode_block(der.as_slice());
            der_certs.insert(key.to_string(), der);
        }
        let val = serde_json::to_string_pretty(&der_certs)?;
        let mut f = OpenOptions::new()
            .append(false)
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.trusted_certs_location)?;
        f.write_all(val.as_bytes())?;

        let new_store = create_cert_store(certs);
        *self.cert_store.lock().expect("cert store lock poisoned") = Arc::new(new_store);
        Ok(())
    }

    pub fn get_cert_store(&self) -> Arc<X509Store> {
        let t = self.cert_store.lock().expect("cert store lock poisoned");
        t.clone()
    }

    pub fn get_trusted_certs(&self) -> Vec<X509> {
        let certs = parse_trusted_certs(&self.trusted_certs_location);
        certs.into_values().collect()
    }

    fn write_connections_to_disk(&self) -> Result<(), Error> {
        let c = self.con_cache.lock().expect("connection cache lock poisoned");
        let val = serde_json::to_string_pretty(c.deref())?;
        let mut f = OpenOptions::new()
            .append(false)
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.con_location)
            .map_err(|e| {
                println!("unable to open file for writing: {}", e);
                Error::new(e)
            })?;
        f.write_all(val.as_bytes())?;
        Ok(())
    }

    pub fn update_last_connected(&self, id: &str) -> Result<(), Error> {
        let mut cache = self.con_cache.lock().expect("connection cache lock poisoned");
        if let Some(entry) = cache.get(id) {
            let mut updated = (**entry).clone();
            updated.last_connected = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock is before UNIX epoch")
                    .as_millis() as i64,
            );
            cache.insert(id.to_string(), Arc::new(updated));
        }
        drop(cache);
        self.write_connections_to_disk()?;
        Ok(())
    }

    pub fn get_all_groups(&self) -> Result<HashSet<String>, Error> {
        let connections = self.con_cache
            .lock()
            .expect("connection cache lock poisoned");

        let mut groups: HashSet<String> = HashSet::new();

        // Ensure default group
        groups.insert(get_default_group());

        let collected_groups: HashSet<String> = connections
            .values()
            .map(|connection_entry| connection_entry.group.clone())  // extract the property
            .collect();

        groups.extend(collected_groups);

        Ok(groups)
    }

    fn prepare_managed_icon(&self, ce: &mut ConnectionEntry) -> Result<(), Error> {
        let icon = ce.icon.trim();
        if icon.is_empty() {
            self.remove_managed_icon(&ce.id)?;
            ce.icon = String::new();
            return Ok(());
        }

        let source = Path::new(icon);
        if !source.is_file() {
            return Ok(());
        }

        let extension = source
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_else(|| String::from("png"));

        let destination = self.icons_dir.join(format!("{}.{}", ce.id, extension));
        if source == destination {
            ce.icon = destination.to_string_lossy().to_string();
            return Ok(());
        }

        self.remove_managed_icon(&ce.id)?;
        fs::copy(source, &destination)?;
        ce.icon = destination.to_string_lossy().to_string();
        Ok(())
    }

    fn remove_managed_icon(&self, id: &str) -> Result<(), Error> {
        if !self.icons_dir.exists() {
            return Ok(());
        }

        for entry in fs::read_dir(&self.icons_dir)? {
            let entry = entry?;
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if stem == id && path.is_file() {
                fs::remove_file(path)?;
            }
        }

        Ok(())
    }

    fn connection_entry_json(&self, ce: &ConnectionEntry) -> Value {
        let mut value = serde_json::to_value(ce).unwrap_or(Value::Null);
        if let Value::Object(ref mut map) = value {
            map.insert(
                String::from("iconDataUrl"),
                match icon_data_url(&ce.icon) {
                    Some(data_url) => Value::String(data_url),
                    None => Value::Null,
                },
            );
        }
        value
    }

    fn exportable_connection_entry(&self, ce: &ConnectionEntry) -> Value {
        let mut exported = ce.clone();
        exported.username = None;
        exported.password = None;
        exported.last_connected = None;

        let mut value = serde_json::to_value(exported).unwrap_or(Value::Null);
        if let Value::Object(ref mut map) = value {
            map.insert(String::from("icon"), Value::String(String::new()));
            map.insert(
                String::from("iconDataUrl"),
                match icon_data_url(&ce.icon) {
                    Some(data_url) => Value::String(data_url),
                    None => Value::Null,
                },
            );
        }
        value
    }

    fn import_preview_entry(&self, entry: &ImportedConnectionEntry) -> Value {
        let mut preview = entry.connection.clone();
        preview.username = None;
        preview.password = None;
        preview.last_connected = None;
        if preview.id.trim().is_empty() {
            preview.id = Uuid::new_v4().to_string();
        }

        let mut value = serde_json::to_value(preview).unwrap_or(Value::Null);
        if let Value::Object(ref mut map) = value {
            map.insert(
                String::from("iconDataUrl"),
                match entry.icon_data_url.as_deref() {
                    Some(data_url) => Value::String(data_url.to_string()),
                    None => match icon_data_url(&entry.connection.icon) {
                        Some(data_url) => Value::String(data_url),
                        None => Value::Null,
                    },
                },
            );
        }
        value
    }

    fn read_import_entries(file_path: &str) -> Result<Vec<ImportedConnectionEntry>, Error> {
        let f = File::open(file_path)?;
        let data: Vec<ImportedConnectionEntry> = serde_json::from_reader(f)?;
        Ok(data)
    }

    fn write_managed_icon_data_url(&self, id: &str, data_url: &str) -> Result<String, Error> {
        let Some((mime_type, encoded)) = data_url
            .strip_prefix("data:")
            .and_then(|value| value.split_once(";base64,"))
        else {
            self.remove_managed_icon(id)?;
            return Ok(String::new());
        };

        let extension = extension_for_mime_type(mime_type);
        let destination = self.icons_dir.join(format!("{}.{}", id, extension));
        self.remove_managed_icon(id)?;
        let bytes = openssl::base64::decode_block(encoded)?;
        fs::write(&destination, bytes)?;
        Ok(destination.to_string_lossy().to_string())
    }
}

fn export_order_key(value: &Value) -> (i64, i64, i64, String) {
    let group_order = value
        .get("groupOrder")
        .and_then(|value| value.as_i64())
        .unwrap_or_default();
    let environment_order = value
        .get("environmentOrder")
        .and_then(|value| value.as_i64())
        .unwrap_or_default();
    let sort_order = value
        .get("sortOrder")
        .and_then(|value| value.as_i64())
        .unwrap_or_default();
    let name = value
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();

    (group_order, environment_order, sort_order, name)
}

fn icon_data_url(icon_path: &str) -> Option<String> {
    let trimmed = icon_path.trim();
    if trimmed.is_empty() {
        return None;
    }

    let bytes = fs::read(trimmed).ok()?;
    let mime_type = match trimmed
        .rsplit('.')
        .next()
        .map(|ext| ext.to_ascii_lowercase())
    {
        Some(ext) if ext == "png" => "image/png",
        Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg",
        Some(ext) if ext == "gif" => "image/gif",
        Some(ext) if ext == "icns" => "image/icns",
        _ => "application/octet-stream",
    };

    let encoded = openssl::base64::encode_block(&bytes);
    Some(format!("data:{};base64,{}", mime_type, encoded))
}

fn extension_for_mime_type(mime_type: &str) -> &'static str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/icns" => "icns",
        _ => "png",
    }
}

pub fn find_java_home() -> String {
    let mut java_home = String::from("");
    if let Some(jh) = OS_ENV.var_os("JAVA_HOME") {
        if let Some(jh_str) = jh.to_str() {
            java_home = String::from(jh_str);
            println!("JAVA_HOME is set to {}", java_home);
        } else {
            println!("JAVA_HOME contains non-UTF-8 characters, ignoring");
        }
    }

    if java_home.is_empty() {
        let out = Command::new("/usr/libexec/java_home")
            .args(["-v", "1.8"])
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                match String::from_utf8(out.stdout) {
                    Ok(jh) => {
                        println!("/usr/libexec/java_home -v 1.8 returned {}", jh);
                        java_home = jh;
                    }
                    Err(e) => {
                        println!("java_home output was not valid UTF-8: {}", e);
                    }
                }
            }
        }
    }
    java_home
}

fn parse_trusted_certs(trusted_certs_location: &PathBuf) -> FxHashMap<String, X509> {
    let mut certs = FxHashMap::default();
    let trusted_certs_location_file = File::open(trusted_certs_location);
    if let Ok(trusted_certs_location_file) = trusted_certs_location_file {
        let cert_map: serde_json::Result<FxHashMap<String, String>> =
            serde_json::from_reader(trusted_certs_location_file);
        if let Ok(cert_map) = cert_map {
            for (key, der_data) in cert_map {
                let der_data = openssl::base64::decode_block(&der_data);
                if let Ok(der_data) = der_data {
                    let c = X509::from_der(der_data.as_slice());
                    if let Ok(c) = c {
                        certs.insert(key, c);
                    } else {
                        println!(
                            "failed to parse cert from DER data with key {} {:?}",
                            key,
                            c.err()
                        );
                    }
                } else {
                    println!(
                        "invalid base64 encoded data with key {} {:?}",
                        key,
                        der_data.err()
                    );
                }
            }
        } else {
            println!(
                "failed to parse trusted certificates JSON file {:?} {:?}",
                trusted_certs_location,
                cert_map.err()
            );
        }
    }

    println!("found {} trusted certificates", certs.len());
    certs
}

fn create_cert_store(certs: FxHashMap<String, X509>) -> X509Store {
    if !openssl_probe::has_ssl_cert_env_vars() {
        println!("probing and setting OpenSSL environment variables");
        // SAFETY: must be called before any OpenSSL operations to set cert paths
        unsafe { openssl_probe::init_openssl_env_vars(); }
    }
    let mut cert_store_builder =
        X509StoreBuilder::new().expect("unable to created X509 store builder");
    cert_store_builder
        .set_default_paths()
        .expect("failed to load system default trusted certs");
    for (_, c) in certs {
        cert_store_builder
            .add_cert(c)
            .expect("failed to add a cert to the in-memory store");
    }

    cert_store_builder.build()
}

fn get_verify() -> bool {
    //println!("getting default value for verify attribute");
    true
}

fn get_default_group() -> String {
    String::from("Default")
}

fn get_default_notes() -> String {
    String::from("")
}

fn get_default_environment() -> String {
    String::from("")
}

fn get_default_donotcache() -> bool {
    false
}
