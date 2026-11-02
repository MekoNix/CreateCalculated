//! Downloads the game's client JAR from Mojang, NeoForge from its Maven repository and mods
//! from Modrinth into a local cache.
//! Every file is checked against the hashes those APIs publish. The JARs are data for
//! the renderer and are never run.

use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use sha2::Sha512;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const MODRINTH: &str = "https://api.modrinth.com/v2";
const NEOFORGE: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";
const USER_AGENT: &str = concat!("MekoNix/CreateCalculated/", env!("CARGO_PKG_VERSION"), " (github.com/MekoNix/CreateCalculated)");
const MAX_JSON: u64 = 64 * 1024 * 1024;
const MAX_JAR: u64 = 1024 * 1024 * 1024;

/// The newest stable NeoForge for a game version in Maven metadata. Game 1.21.1 maps to 21.1.x.
fn pick_neoforge(metadata: &str, game: &str) -> Result<String, String> {
    let prefix = format!("{}.", game.strip_prefix("1.").ok_or_else(|| format!("no NeoForge scheme for game {game}"))?);
    let number = |v: &str| v.strip_prefix(&prefix)?.parse::<u64>().ok();
    metadata
        .split("<version>")
        .skip(1)
        .filter_map(|part| part.split('<').next())
        .filter(|v| number(v).is_some() && safe_file_name(&format!("neoforge-{v}-universal.jar")).is_some())
        .max_by_key(|v| number(v))
        .map(String::from)
        .ok_or_else(|| format!("NeoForge has no version for game {game}"))
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Hashes {
    sha1: Option<String>,
    sha512: Option<String>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compares what was computed with what the API said. An API that gave no hash at all
/// proves nothing, so that counts as a failure too.
fn check_hashes(name: &str, expected: &Hashes, sha1: &str, sha512: &str) -> Result<(), String> {
    if expected.sha1.is_none() && expected.sha512.is_none() {
        return Err(format!("{name}: the API gave no hash to check against"));
    }
    if expected.sha1.as_deref().is_some_and(|h| !h.eq_ignore_ascii_case(sha1)) {
        return Err(format!("{name}: sha1 mismatch"));
    }
    if expected.sha512.as_deref().is_some_and(|h| !h.eq_ignore_ascii_case(sha512)) {
        return Err(format!("{name}: sha512 mismatch"));
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<(String, String), String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (mut a, mut b) = (Sha1::new(), Sha512::new());
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        a.update(&buffer[..n]);
        b.update(&buffer[..n]);
    }
    Ok((hex(&a.finalize()), hex(&b.finalize())))
}

fn is_slug(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('.') && s.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

/// A file name from an API response, reduced to something safe inside the cache folder.
fn safe_file_name(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next()?;
    // The name of a language file cannot match downloads.json, the record kept in this folder.
    let clean = !base.starts_with('.') && (base.ends_with(".jar") || base.ends_with(".lang.json")) && base.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'+'));
    clean.then(|| base.to_string())
}

/// The newest stable file of a project's version list, or the version asked for.
fn pick_modrinth_file(versions: &Value, pin: Option<&str>) -> Result<(String, String, Hashes, String), String> {
    let mut list: Vec<&Value> = versions.as_array().ok_or("unexpected answer from Modrinth")?.iter().collect();
    list.sort_by_key(|v| std::cmp::Reverse(v.get("date_published").and_then(Value::as_str).unwrap_or("")));
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let chosen = match pin {
        Some(pin) => list.iter().find(|v| {
            let number = text(v, "version_number");
            number == pin || text(v, "id") == pin || number.split(['+', '-']).any(|part| part == pin)
        }),
        None => list.iter().find(|v| text(v, "version_type") == "release").or(list.first()),
    };
    let version = chosen.ok_or_else(|| match pin {
        Some(pin) => format!("no version {pin}"),
        None => "no version for this game version and loader".to_string(),
    })?;
    let files = version.get("files").and_then(Value::as_array).ok_or("version without files")?;
    let file = files.iter().find(|f| f.get("primary").and_then(Value::as_bool) == Some(true)).or(files.first()).ok_or("version without files")?;
    let hashes = Hashes { sha1: file.pointer("/hashes/sha1").and_then(Value::as_str).map(String::from), sha512: file.pointer("/hashes/sha512").and_then(Value::as_str).map(String::from) };
    Ok((text(file, "url"), text(file, "filename"), hashes, text(version, "version_number")))
}

pub struct Fetcher {
    agent: ureq::Agent,
    dir: PathBuf,
    /// What was downloaded before: request key to file and hashes.
    records: BTreeMap<String, Value>,
    /// Ask the APIs again even when a cached file would do.
    pub update: bool,
}

impl Fetcher {
    pub fn new(dir: &Path) -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(Duration::from_secs(600)))
            .https_only(true)
            .build();
        let records = std::fs::read(dir.join("downloads.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_object().map(|o| o.clone().into_iter().collect()))
            .unwrap_or_default();
        Self { agent: config.new_agent(), dir: dir.to_path_buf(), records, update: false }
    }

    fn save_records(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&self.records).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join("downloads.json"), text).map_err(|e| format!("downloads.json: {e}"))
    }

    fn get_text(&self, url: &str) -> Result<String, String> {
        let mut response = self.agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
        response.body_mut().with_config().limit(MAX_JSON).read_to_string().map_err(|e| format!("{url}: {e}"))
    }

    fn get_json(&self, url: &str) -> Result<Value, String> {
        serde_json::from_str(&self.get_text(url)?).map_err(|e| format!("{url}: {e}"))
    }

    /// A previous download of this request, if its file is still there and intact.
    fn cached(&self, key: &str) -> Option<PathBuf> {
        let record = self.records.get(key)?;
        let path = self.dir.join(safe_file_name(record.get("file")?.as_str()?)?);
        let (sha1, sha512) = hash_file(&path).ok()?;
        (record.get("sha1").and_then(Value::as_str) == Some(sha1.as_str()) && record.get("sha512").and_then(Value::as_str) == Some(sha512.as_str())).then_some(path)
    }

    fn download(&mut self, key: &str, url: &str, file_name: &str, expected: &Hashes) -> Result<PathBuf, String> {
        if !url.starts_with("https://") {
            return Err(format!("{url}: refusing a download that is not https"));
        }
        let file_name = safe_file_name(file_name).ok_or_else(|| format!("unsafe file name {file_name}"))?;
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let target = self.dir.join(&file_name);
        let part = self.dir.join(format!("{file_name}.part"));

        let existing = hash_file(&target).ok().filter(|(a, b)| check_hashes(&file_name, expected, a, b).is_ok());
        let (sha1, sha512) = match existing {
            Some(hashes) => hashes,
            None => {
                println!("downloading {url}");
                let result = self.stream_to(url, &part).and_then(|(a, b)| {
                    check_hashes(&file_name, expected, &a, &b)?;
                    Ok((a, b))
                });
                match result {
                    Ok(hashes) => {
                        std::fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))?;
                        hashes
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&part);
                        return Err(e);
                    }
                }
            }
        };
        self.records.insert(key.to_string(), json!({ "file": file_name, "sha1": sha1, "sha512": sha512 }));
        self.save_records()?;
        Ok(target)
    }

    fn stream_to(&self, url: &str, part: &Path) -> Result<(String, String), String> {
        let response = self.agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
        let mut reader = response.into_body().into_with_config().limit(MAX_JAR).reader();
        let mut file = File::create(part).map_err(|e| format!("{}: {e}", part.display()))?;
        let (mut a, mut b) = (Sha1::new(), Sha512::new());
        let mut buffer = vec![0; 256 * 1024];
        loop {
            let n = reader.read(&mut buffer).map_err(|e| format!("{url}: {e}"))?;
            if n == 0 {
                break;
            }
            a.update(&buffer[..n]);
            b.update(&buffer[..n]);
            file.write_all(&buffer[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
        }
        file.flush().map_err(|e| format!("{}: {e}", part.display()))?;
        Ok((hex(&a.finalize()), hex(&b.finalize())))
    }

    /// The game's client JAR, which holds the textures and models.
    pub fn minecraft(&mut self, version: &str) -> Result<PathBuf, String> {
        if !is_slug(version) {
            return Err(format!("bad game version {version}"));
        }
        let key = format!("minecraft:{version}");
        if let Some(path) = self.cached(&key) {
            return Ok(path);
        }
        let manifest = self.get_json(MANIFEST)?;
        let entry = manifest
            .get("versions")
            .and_then(Value::as_array)
            .and_then(|list| list.iter().find(|v| v.get("id").and_then(Value::as_str) == Some(version)))
            .ok_or_else(|| format!("Mojang has no version {version}"))?;
        let url = entry.get("url").and_then(Value::as_str).ok_or("manifest entry without url")?;
        let details = self.get_json(url)?;
        let client = details.pointer("/downloads/client").ok_or("version without a client download")?;
        let url = client.get("url").and_then(Value::as_str).ok_or("client download without url")?.to_string();
        let expected = Hashes { sha1: client.get("sha1").and_then(Value::as_str).map(String::from), sha512: None };
        self.download(&key, &url, &format!("minecraft-{version}-client.jar"), &expected)
    }

    /// NeoForge, whose JAR holds the common `c:` tags that recipes refer to.
    /// A language file of the game. Only English is inside the JAR, the others are assets.
    pub fn minecraft_lang(&mut self, version: &str, lang: &str) -> Result<PathBuf, String> {
        if !is_slug(version) || !is_slug(lang) {
            return Err(format!("bad game version or language {version} {lang}"));
        }
        let key = format!("minecraft-lang:{version}:{lang}");
        if let Some(path) = self.cached(&key) {
            return Ok(path);
        }
        let manifest = self.get_json(MANIFEST)?;
        let entry = manifest
            .get("versions")
            .and_then(Value::as_array)
            .and_then(|list| list.iter().find(|v| v.get("id").and_then(Value::as_str) == Some(version)))
            .ok_or_else(|| format!("Mojang has no version {version}"))?;
        let url = entry.get("url").and_then(Value::as_str).ok_or("manifest entry without url")?;
        let details = self.get_json(url)?;
        let index = details.pointer("/assetIndex/url").and_then(Value::as_str).ok_or("version without an asset index")?;
        let assets = self.get_json(index)?;
        let name = format!("minecraft/lang/{lang}.json");
        let hash = assets
            .get("objects")
            .and_then(|o| o.get(&name))
            .and_then(|o| o.get("hash"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("the game has no {name}"))?
            .to_string();
        let url = format!("https://resources.download.minecraft.net/{}/{hash}", hash.get(..2).ok_or("bad asset hash")?);
        let expected = Hashes { sha1: Some(hash), sha512: None };
        self.download(&key, &url, &format!("minecraft-{version}-{lang}.lang.json"), &expected)
    }

    pub fn neoforge(&mut self, game: &str) -> Result<PathBuf, String> {
        if !is_slug(game) {
            return Err(format!("bad game version {game}"));
        }
        let key = format!("neoforge:{game}");
        if !self.update {
            if let Some(path) = self.cached(&key) {
                return Ok(path);
            }
        }
        let version = pick_neoforge(&self.get_text(&format!("{NEOFORGE}/maven-metadata.xml"))?, game)?;
        println!("neoforge: version {version}");
        let file_name = format!("neoforge-{version}-universal.jar");
        let url = format!("{NEOFORGE}/{version}/{file_name}");
        let sha1 = self.get_text(&format!("{url}.sha1"))?.trim().to_string();
        self.download(&key, &url, &file_name, &Hashes { sha1: Some(sha1), sha512: None })
    }

    /// A mod from Modrinth, `slug` or `slug@version`.
    pub fn modrinth(&mut self, spec: &str, game: &str, loader: &str) -> Result<PathBuf, String> {
        let (slug, pin) = match spec.split_once('@') {
            Some((slug, pin)) => (slug, Some(pin)),
            None => (spec, None),
        };
        if !is_slug(slug) || !is_slug(game) || !is_slug(loader) {
            return Err(format!("bad mod request {spec}"));
        }
        let key = format!("modrinth:{spec}:{game}:{loader}");
        if !self.update {
            if let Some(path) = self.cached(&key) {
                return Ok(path);
            }
        }
        let url = format!("{MODRINTH}/project/{slug}/version?game_versions=%5B%22{game}%22%5D&loaders=%5B%22{loader}%22%5D");
        let versions = self.get_json(&url)?;
        let (url, file_name, expected, number) = pick_modrinth_file(&versions, pin).map_err(|e| format!("{slug}: {e}"))?;
        println!("{slug}: version {number}");
        self.download(&key, &url, &file_name, &expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hashes(sha1: &str, sha512: &str) -> Hashes {
        Hashes { sha1: Some(sha1.into()), sha512: Some(sha512.into()) }
    }

    #[test]
    fn hash_checks_fail_on_any_mismatch() {
        assert!(check_hashes("x", &hashes("AB", "cd"), "ab", "cd").is_ok());
        assert!(check_hashes("x", &hashes("ab", "cd"), "00", "cd").unwrap_err().contains("sha1"));
        assert!(check_hashes("x", &hashes("ab", "cd"), "ab", "00").unwrap_err().contains("sha512"));
        assert!(check_hashes("x", &Hashes::default(), "ab", "cd").is_err());
    }

    #[test]
    fn neoforge_version_is_the_newest_stable_for_the_game() {
        let xml = "<versions><version>21.1.9</version><version>21.1.265</version><version>21.1.100</version>\
            <version>21.1.300-beta</version><version>21.4.1</version><version>20.1.5</version></versions>";
        assert_eq!(pick_neoforge(xml, "1.21.1").unwrap(), "21.1.265");
        assert_eq!(pick_neoforge(xml, "1.21.4").unwrap(), "21.4.1");
        assert!(pick_neoforge(xml, "1.19.2").is_err());
        assert!(pick_neoforge(xml, "26.1").is_err());
    }

    #[test]
    fn digests_match_known_values() {
        assert_eq!(hex(&Sha1::digest(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert!(hex(&Sha512::digest(b"abc")).starts_with("ddaf35a193617aba"));
    }

    #[test]
    fn file_names_from_apis_cannot_leave_the_folder() {
        assert_eq!(safe_file_name("create-1.21.1-6.0.10.jar").as_deref(), Some("create-1.21.1-6.0.10.jar"));
        assert_eq!(safe_file_name("../../evil.jar").as_deref(), Some("evil.jar"));
        assert_eq!(safe_file_name("a b.jar"), None);
        assert_eq!(safe_file_name(".hidden.jar"), None);
        assert_eq!(safe_file_name("x.exe"), None);
        assert_eq!(safe_file_name("downloads.json"), None);
        assert_eq!(safe_file_name("minecraft-1.21.1-ru_ru.lang.json").as_deref(), Some("minecraft-1.21.1-ru_ru.lang.json"));
    }

    #[test]
    fn modrinth_prefers_the_newest_release_unless_pinned() {
        let list = json!([
            {"id":"a","version_number":"7.0-beta","version_type":"beta","date_published":"2026-05-01","files":[{"url":"https://x/a.jar","filename":"a.jar","primary":true,"hashes":{"sha1":"1"}}]},
            {"id":"b","version_number":"6.0.10+mc1.21.1","version_type":"release","date_published":"2026-04-01","files":[
                {"url":"https://x/sources.jar","filename":"sources.jar","primary":false,"hashes":{"sha1":"2"}},
                {"url":"https://x/b.jar","filename":"b.jar","primary":true,"hashes":{"sha1":"3","sha512":"4"}}]},
            {"id":"c","version_number":"mc1.21.1-6.0.9","version_type":"release","date_published":"2026-01-01","files":[{"url":"https://x/c.jar","filename":"c.jar","primary":true,"hashes":{"sha1":"5"}}]}
        ]);
        let (url, name, hash, number) = pick_modrinth_file(&list, None).unwrap();
        assert_eq!((url.as_str(), name.as_str(), number.as_str()), ("https://x/b.jar", "b.jar", "6.0.10+mc1.21.1"));
        assert_eq!(hash, hashes("3", "4"));
        assert_eq!(pick_modrinth_file(&list, Some("6.0.9")).unwrap().1, "c.jar");
        assert_eq!(pick_modrinth_file(&list, Some("6.0.10")).unwrap().1, "b.jar");
        assert!(pick_modrinth_file(&list, Some("9.9")).is_err());
        assert!(pick_modrinth_file(&json!([]), None).is_err());
    }
}
