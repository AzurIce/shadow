use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId(String);

impl ObjectId {
    pub fn from_sha256_hex(hex: impl Into<String>) -> Result<Self> {
        let hex = hex.into();
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("invalid SHA-256 digest");
        }
        Ok(Self(hex.to_ascii_lowercase()))
    }

    pub fn parse(value: &str) -> Result<Self> {
        let hex = value
            .strip_prefix("sha256:")
            .context("object id must start with 'sha256:'")?;
        Self::from_sha256_hex(hex)
    }

    pub fn hex(&self) -> &str {
        &self.0
    }

    pub fn as_ref_value(&self) -> String {
        format!("sha256:{}", self.0)
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShadowRef {
    pub oid: ObjectId,
    pub size: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct RefDocument {
    version: u32,
    oid: String,
    size: u64,
}

impl ShadowRef {
    pub fn parse(content: &str) -> Result<Self> {
        let document: RefDocument = toml::from_str(content).context("invalid ref TOML")?;
        if document.version != 1 {
            bail!("unsupported ref version: {}", document.version);
        }
        Ok(Self {
            oid: ObjectId::parse(&document.oid)?,
            size: document.size,
        })
    }

    pub fn serialize(&self) -> Result<String> {
        toml::to_string(&RefDocument {
            version: 1,
            oid: self.oid.as_ref_value(),
            size: self.size,
        })
        .context("failed to serialize ref")
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlobKey(String);

impl BlobKey {
    pub fn for_object(repository_id: &str, oid: &ObjectId) -> Self {
        Self(format!(
            "repositories/{}/objects/sha256/{}/{}",
            repository_id,
            &oid.hex()[..2],
            &oid.hex()[2..]
        ))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlobMetadata {
    pub size: u64,
    pub etag: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_object_ids() {
        let oid = ObjectId::from_sha256_hex("a".repeat(64)).unwrap();
        assert_eq!(oid.to_string(), format!("sha256:{}", "a".repeat(64)));
        assert!(ObjectId::parse("md5:abc").is_err());
    }

    #[test]
    fn ref_round_trip() {
        let reference = ShadowRef {
            oid: ObjectId::from_sha256_hex("b".repeat(64)).unwrap(),
            size: 42,
        };
        assert_eq!(
            ShadowRef::parse(&reference.serialize().unwrap()).unwrap(),
            reference
        );
    }
}
