//! Verified Antigravity wire metadata (`Start.md` §10, §16, §18).
//! Field numbers come from the installed 2.24.0 protobuf descriptors. Message
//! bodies remain opaque: skip their wire payloads, preserve other map entries.
use super::*;
use std::io::{Read, Seek, SeekFrom};

#[derive(Debug)]
pub(super) struct Field<'a> {
    pub number: u64,
    pub wire: u8,
    pub payload: &'a [u8],
    pub raw: &'a [u8],
}
fn varint(bytes: &[u8], index: &mut usize) -> anyhow::Result<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes
            .get(*index)
            .ok_or_else(|| anyhow::anyhow!("truncated protobuf varint"))?;
        *index += 1;
        anyhow::ensure!(shift != 63 || byte <= 1, "overflowing protobuf varint");
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    anyhow::bail!("invalid protobuf varint")
}
pub(super) fn fields(bytes: &[u8]) -> anyhow::Result<Vec<Field<'_>>> {
    let mut result = vec![];
    let mut i = 0;
    while i < bytes.len() {
        let begin = i;
        let tag = varint(bytes, &mut i)?;
        let wire = (tag & 7) as u8;
        anyhow::ensure!(tag >> 3 != 0, "invalid protobuf field");
        let size = match wire {
            0 => {
                let begin = i;
                varint(bytes, &mut i)?;
                i - begin
            }
            1 => 8,
            2 => usize::try_from(varint(bytes, &mut i)?)?,
            5 => 4,
            _ => anyhow::bail!("unsupported protobuf wire type"),
        };
        let payload = if wire == 0 {
            &bytes[i - size..i]
        } else {
            let end = i
                .checked_add(size)
                .ok_or_else(|| anyhow::anyhow!("protobuf length overflow"))?;
            let p = bytes
                .get(i..end)
                .ok_or_else(|| anyhow::anyhow!("truncated protobuf payload"))?;
            i = end;
            p
        };
        result.push(Field {
            number: tag >> 3,
            wire,
            payload,
            raw: &bytes[begin..i],
        });
    }
    Ok(result)
}

/// Preserve every neighboring summary byte-for-byte, including unknown fields
/// inside its value. Unknown outer structure is not safe to edit.
pub(super) fn remove_summary(bytes: &[u8], id: &str) -> anyhow::Result<Vec<u8>> {
    let mut result = vec![];
    let mut seen = HashSet::new();
    let mut found = false;
    for field in fields(bytes)? {
        anyhow::ensure!(
            field.number == 1 && field.wire == 2,
            "unsupported summaries map structure"
        );
        let entry = fields(field.payload)?;
        anyhow::ensure!(
            entry.len() == 2
                && entry.iter().all(|f| f.wire == 2)
                && entry.iter().any(|f| f.number == 2),
            "unsupported summaries map entry"
        );
        let key = entry
            .iter()
            .find(|f| f.number == 1)
            .ok_or_else(|| anyhow::anyhow!("summary entry lacks key"))?;
        let key = std::str::from_utf8(key.payload)?;
        anyhow::ensure!(
            valid_id(key) && seen.insert(key.to_owned()),
            "invalid/duplicate summary key"
        );
        if key == id {
            found = true;
        } else {
            result.extend_from_slice(field.raw);
        }
    }
    // Older empty cache snapshots can omit an indexed conversation. A missing
    // target is an idempotent map edit, never permission to invent other keys.
    let _ = found;
    Ok(result)
}

fn read_varint(file: &mut fs::File) -> anyhow::Result<Option<u64>> {
    let mut bytes = vec![];
    for _ in 0..10 {
        let mut byte = [0];
        if file.read(&mut byte)? == 0 {
            anyhow::ensure!(bytes.is_empty(), "truncated protobuf varint");
            return Ok(None);
        }
        bytes.push(byte[0]);
        if byte[0] < 128 {
            return Ok(Some(varint(&bytes, &mut 0)?));
        }
    }
    anyhow::bail!("invalid protobuf varint")
}

/// Verify legacy trajectory identity without loading a single step body.
pub(super) fn legacy_owned(path: &Path, id: &str) -> anyhow::Result<bool> {
    let mut file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    let mut cascade = None;
    let mut independent = true;
    while let Some(tag) = read_varint(&mut file)? {
        let number = tag >> 3;
        if !(1..=10).contains(&number) {
            independent = false;
        }
        match tag & 7 {
            0 => {
                read_varint(&mut file)?.ok_or_else(|| anyhow::anyhow!("missing scalar"))?;
            }
            1 => {
                file.seek(SeekFrom::Current(8))?;
            }
            5 => {
                file.seek(SeekFrom::Current(4))?;
            }
            2 => {
                let size =
                    read_varint(&mut file)?.ok_or_else(|| anyhow::anyhow!("missing length"))?;
                anyhow::ensure!(
                    file.stream_position()?
                        .checked_add(size)
                        .is_some_and(|end| end <= length),
                    "truncated trajectory"
                );
                if number == 6 {
                    anyhow::ensure!(
                        size <= 128 && cascade.is_none(),
                        "invalid/duplicate cascade id"
                    );
                    let mut data = vec![0; size as usize];
                    file.read_exact(&mut data)?;
                    cascade = Some(String::from_utf8(data)?);
                } else if number == 5 {
                    independent = false;
                    file.seek(SeekFrom::Current(i64::try_from(size)?))?;
                } else if number == 7 {
                    anyhow::ensure!(size <= 2 * 1024 * 1024, "trajectory metadata too large");
                    let mut data = vec![0; size as usize];
                    file.read_exact(&mut data)?;
                    for f in fields(&data)? {
                        if ![
                            1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                        ]
                        .contains(&f.number)
                        {
                            independent = false;
                        }
                        if f.number == 5 && !f.payload.is_empty() {
                            independent = false;
                        }
                        if f.number == 6 && f.payload != id.as_bytes() {
                            independent = false;
                        }
                        if [8, 16, 17, 22, 23].contains(&f.number) {
                            independent = false;
                        }
                    }
                } else {
                    file.seek(SeekFrom::Current(i64::try_from(size)?))?;
                }
            }
            _ => anyhow::bail!("unsupported trajectory wire type"),
        }
        anyhow::ensure!(
            file.stream_position()? <= length,
            "truncated trajectory scalar"
        );
    }
    Ok(independent && cascade.as_deref() == Some(id))
}

/// SQLite trajectories carry explicit id/parent-reference facts. No step payload
/// is queried. Unrecognized parent relationships remain uncleanable.
pub(super) fn sqlite_owned(path: &Path, id: &str) -> anyhow::Result<bool> {
    let db = ReadOnlyDb::open(path)?;
    let mut stmt = db
        .connection()
        .prepare("SELECT cascade_id FROM trajectory_meta")?;
    let ids = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let parents: i64 =
        db.connection()
            .query_row("SELECT COUNT(*) FROM parent_references", [], |r| r.get(0))?;
    let metadata: Option<Vec<u8>> = db
        .connection()
        .query_row(
            "SELECT data FROM trajectory_metadata_blob WHERE id='main'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let mut independent = parents == 0;
    if let Some(data) = metadata {
        for f in fields(&data)? {
            if (f.number == 5 && !f.payload.is_empty())
                || (f.number == 6 && f.payload != id.as_bytes())
                || [8, 16, 17, 22, 23].contains(&f.number)
            {
                independent = false;
            }
        }
    }
    Ok(ids == [id] && independent)
}
use rusqlite::OptionalExtension;

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Atom(String),
    Quoted(String),
    Message(Vec<(String, Value)>),
}
fn token(bytes: &[u8], i: &mut usize) -> anyhow::Result<Option<String>> {
    loop {
        while bytes.get(*i).is_some_and(u8::is_ascii_whitespace) {
            *i += 1;
        }
        if bytes.get(*i) == Some(&b'#') {
            while bytes.get(*i).is_some_and(|c| *c != b'\n') {
                *i += 1;
            }
        } else {
            break;
        }
    }
    let start = *i;
    let Some(&c) = bytes.get(*i) else {
        return Ok(None);
    };
    *i += 1;
    if b"{}:<>[],;".contains(&c) {
        return Ok(Some((c as char).to_string()));
    }
    if c == b'"' || c == b'\'' {
        loop {
            let x = *bytes
                .get(*i)
                .ok_or_else(|| anyhow::anyhow!("unterminated protobuf string"))?;
            *i += 1;
            if x == c {
                break;
            }
            if x == b'\\' {
                anyhow::ensure!(*i < bytes.len(), "invalid protobuf escape");
                *i += 1;
            }
        }
    } else {
        while bytes
            .get(*i)
            .is_some_and(|x| !x.is_ascii_whitespace() && !b"{}:<>[],;\"'".contains(x))
        {
            *i += 1;
        }
    }
    Ok(Some(std::str::from_utf8(&bytes[start..*i])?.into()))
}
fn message(tokens: &[String], i: &mut usize, nested: bool) -> anyhow::Result<Vec<(String, Value)>> {
    let mut values = vec![];
    while let Some(name) = tokens.get(*i) {
        if name == "}" || name == ">" {
            anyhow::ensure!(nested, "unmatched protobuf brace");
            *i += 1;
            return Ok(values);
        }
        anyhow::ensure!(
            name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "invalid annotation field"
        );
        let name = name.clone();
        *i += 1;
        if tokens.get(*i).is_some_and(|s| s == ":") {
            *i += 1;
        }
        let v = tokens
            .get(*i)
            .ok_or_else(|| anyhow::anyhow!("missing annotation value"))?
            .clone();
        *i += 1;
        let value = if v == "{" || v == "<" {
            Value::Message(message(tokens, i, true)?)
        } else if v.starts_with(['\'', '"']) {
            Value::Quoted(v)
        } else {
            anyhow::ensure!(
                !["}", ">", ":", "[", "]"].contains(&v.as_str()),
                "invalid annotation scalar"
            );
            Value::Atom(v)
        };
        values.push((name, value));
        if tokens.get(*i).is_some_and(|s| s == "," || s == ";") {
            *i += 1;
        }
    }
    anyhow::ensure!(!nested, "unclosed protobuf annotation");
    Ok(values)
}
#[derive(Debug, Default)]
pub(super) struct Annotation {
    pub archived: bool,
    pub archived_at: Option<u64>,
    pub title: Option<String>,
}
pub(super) fn annotation(path: &Path) -> anyhow::Result<Annotation> {
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 1024 * 1024,
        "invalid annotation file"
    );
    let bytes = fs::read(path)?;
    let mut tokens = vec![];
    let mut i = 0;
    while let Some(t) = token(&bytes, &mut i)? {
        tokens.push(t);
    }
    let values = message(&tokens, &mut 0, false)?;
    let mut result = Annotation::default();
    let mut seen = HashSet::new();
    for (name, value) in values {
        if ["archived", "archival_status_timestamp", "title"].contains(&name.as_str()) {
            anyhow::ensure!(seen.insert(name.clone()), "duplicate annotation field");
        }
        match (name.as_str(), value) {
            ("archived", Value::Atom(v)) => {
                anyhow::ensure!(
                    ["true", "false"].contains(&v.as_str()),
                    "invalid archived boolean"
                );
                result.archived = v == "true";
            }
            ("archival_status_timestamp", Value::Message(fields)) => {
                let mut seconds = None;
                let mut nanos = 0u64;
                let mut seen = HashSet::new();
                for (key, value) in fields {
                    anyhow::ensure!(seen.insert(key.clone()), "duplicate timestamp field");
                    if let Value::Atom(v) = value {
                        match key.as_str() {
                            "seconds" => seconds = v.parse::<u64>().ok(),
                            "nanos" => nanos = v.parse()?,
                            _ => anyhow::bail!("unknown timestamp field"),
                        }
                    } else {
                        anyhow::bail!("invalid timestamp value");
                    }
                }
                anyhow::ensure!(nanos < 1_000_000_000, "invalid timestamp nanos");
                result.archived_at = seconds
                    .and_then(|s| s.checked_mul(1000))
                    .and_then(|s| s.checked_add(nanos / 1_000_000));
            }
            ("title", Value::Quoted(v)) => {
                result.title = serde_json::from_str(&v).ok();
            }
            ("archived" | "archival_status_timestamp" | "title", _) => {
                anyhow::bail!("invalid annotation value type")
            }
            _ => {}
        }
    }
    Ok(result)
}
