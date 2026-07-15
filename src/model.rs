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
    pub content_type: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct RefDocument {
    version: u32,
    oid: String,
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
}

impl ShadowRef {
    pub fn parse(content: &str) -> Result<Self> {
        let document: RefDocument = toml::from_str(content).context("invalid ref TOML")?;
        if document.version != 1 {
            bail!("unsupported ref version: {}", document.version);
        }
        let content_type = document
            .content_type
            .map(validate_content_type)
            .transpose()?;
        Ok(Self {
            oid: ObjectId::parse(&document.oid)?,
            size: document.size,
            content_type,
        })
    }

    pub fn serialize(&self) -> Result<String> {
        toml::to_string(&RefDocument {
            version: 1,
            oid: self.oid.as_ref_value(),
            size: self.size,
            content_type: self.content_type.clone(),
        })
        .context("failed to serialize ref")
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlobKey(String);

impl BlobKey {
    pub fn for_object(name: &str, oid: &ObjectId) -> Self {
        Self(format!(
            "{}/objects/sha256/{}/{}",
            name,
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
    pub content_type: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UploadOptions {
    pub content_type: String,
}

fn validate_content_type(value: String) -> Result<String> {
    value
        .parse::<mime::Mime>()
        .with_context(|| format!("invalid content type: {value}"))?;
    Ok(value)
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
            content_type: Some("image/png".to_string()),
        };
        assert_eq!(
            ShadowRef::parse(&reference.serialize().unwrap()).unwrap(),
            reference
        );
    }

    #[test]
    fn parses_legacy_ref_without_content_type() {
        let reference = ShadowRef::parse(&format!(
            "version = 1\noid = \"sha256:{}\"\nsize = 42\n",
            "b".repeat(64)
        ))
        .unwrap();
        assert_eq!(reference.content_type, None);
    }

    #[test]
    fn builds_project_scoped_blob_key() {
        let oid = ObjectId::from_sha256_hex("a".repeat(64)).unwrap();
        assert_eq!(
            BlobKey::for_object("my-project", &oid).as_str(),
            format!("my-project/objects/sha256/aa/{}", "a".repeat(62))
        );
    }
}
