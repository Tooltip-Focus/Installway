use serde::{Deserialize, Serialize};

/// What gets embedded in the installer .exe as RCDATA id=2.
///
/// `payload_json` is the exact UTF-8 byte sequence the signature was computed over.
/// The verifier verifies the signature against those bytes, *then* parses
/// `InstallerPayload` from them. This avoids any serializer-determinism trap.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SignedPayload {
    pub payload_json: String,
    pub signature_hex: String,
}
