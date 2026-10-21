//! Ephemeral expression-only issuer registration, replay guard and receipt cache.
//! The registration key comes only from the trusted host process environment.
use hmac::{Hmac, Mac};
use owo_agent_protocol::{
    ExpressionOriginCapsule, ExpressionResponse, EXPRESSION_ORIGIN_KEY_ID,
    EXPRESSION_ORIGIN_PURPOSE,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;
const MAX_RECEIPTS: usize = 256;
struct Receipt {
    fingerprint: [u8; 32],
    expires: Instant,
    result: Option<ExpressionResponse>,
}
pub(crate) struct OriginVerifier {
    key: Option<Zeroizing<[u8; 32]>>,
    receipts: HashMap<String, Receipt>,
    last_wall_ms: u64,
    reaper_started: bool,
}
pub(crate) struct OriginLease {
    nonce: String,
    fingerprint: [u8; 32],
}
pub(crate) enum OriginAdmission {
    Fresh(OriginLease),
    Cached(ExpressionResponse),
}
pub(crate) struct OriginError {
    pub status: u16,
    pub code: &'static str,
}
fn denied(code: &'static str) -> OriginError {
    OriginError { status: 403, code }
}
fn hex_bytes(value: &str) -> Option<Zeroizing<Vec<u8>>> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    (0..32)
        .map(|i| u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()
        .map(Zeroizing::new)
}
impl OriginVerifier {
    pub(crate) fn from_env() -> Self {
        let secret = std::env::var("OWO_EXPRESSION_ORIGIN_KEY")
            .ok()
            .map(Zeroizing::new);
        let key = secret
            .as_ref()
            .and_then(|value| hex_bytes(value))
            .and_then(|bytes| <[u8; 32]>::try_from(bytes.as_slice()).ok())
            .filter(|key| key.iter().any(|b| *b != 0))
            .map(Zeroizing::new);
        Self {
            key,
            receipts: HashMap::new(),
            last_wall_ms: 0,
            reaper_started: false,
        }
    }
    fn prune(&mut self, now: Instant) {
        self.receipts.retain(|_, r| now < r.expires);
    }
    pub(crate) fn start_reaper(shared: &Arc<Mutex<Self>>) {
        let Ok(mut guard) = shared.lock() else {
            return;
        };
        if guard.reaper_started {
            return;
        }
        guard.reaper_started = true;
        let weak = Arc::downgrade(shared);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let Some(shared) = weak.upgrade() else {
                    break;
                };
                match shared.lock() {
                    Ok(mut verifier) => verifier.prune(Instant::now()),
                    Err(_) => break,
                };
            }
        });
    }
    pub(crate) fn begin(
        &mut self,
        capsule: &ExpressionOriginCapsule,
        prompt: &str,
    ) -> Result<OriginAdmission, OriginError> {
        let key = self
            .key
            .as_ref()
            .ok_or_else(|| denied("expression/origin_not_registered"))?;
        if capsule.signature.alg != "HMAC-SHA256"
            || capsule.signature.key_id != EXPRESSION_ORIGIN_KEY_ID
        {
            return Err(denied("expression/origin_signature"));
        }
        let bytes = capsule.payload.canonical_bytes().map_err(denied)?;
        let tag = hex_bytes(&capsule.signature.value)
            .ok_or_else(|| denied("expression/origin_signature"))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_ref())
            .map_err(|_| denied("expression/origin_signature"))?;
        mac.update(EXPRESSION_ORIGIN_PURPOSE);
        mac.update(&bytes);
        mac.verify_slice(&tag)
            .map_err(|_| denied("expression/origin_signature"))?;
        let digest = format!("{:x}", Sha256::digest(prompt.as_bytes()));
        if digest != capsule.payload.prompt_sha256 {
            return Err(denied("expression/origin_prompt_mismatch"));
        }
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| denied("expression/origin_clock"))?
            .as_millis();
        let now_ms = u64::try_from(wall).map_err(|_| denied("expression/origin_clock"))?;
        if now_ms < self.last_wall_ms {
            return Err(denied("expression/origin_clock"));
        }
        self.last_wall_ms = now_ms;
        let freshness = &capsule.payload.freshness;
        let end = freshness
            .issued_at_ms
            .checked_add(freshness.ttl_ms)
            .ok_or_else(|| denied("expression/origin_expired"))?;
        if freshness.issued_at_ms > now_ms.saturating_add(5000) || now_ms >= end {
            return Err(denied("expression/origin_expired"));
        }
        let now = Instant::now();
        self.prune(now);
        let fingerprint: [u8; 32] = Sha256::digest(&bytes).into();
        if let Some(receipt) = self.receipts.get(&capsule.payload.nonce) {
            if receipt.fingerprint != fingerprint {
                return Err(OriginError {
                    status: 409,
                    code: "expression/origin_nonce_conflict",
                });
            }
            return match &receipt.result {
                Some(response) => Ok(OriginAdmission::Cached(response.clone())),
                None => Err(OriginError {
                    status: 409,
                    code: "expression/origin_acceptance_unknown",
                }),
            };
        }
        if self.receipts.len() >= MAX_RECEIPTS {
            return Err(OriginError {
                status: 429,
                code: "expression/origin_busy",
            });
        }
        let expires = now
            .checked_add(Duration::from_millis(end - now_ms))
            .ok_or_else(|| denied("expression/origin_expired"))?;
        self.receipts.insert(
            capsule.payload.nonce.clone(),
            Receipt {
                fingerprint,
                expires,
                result: None,
            },
        );
        Ok(OriginAdmission::Fresh(OriginLease {
            nonce: capsule.payload.nonce.clone(),
            fingerprint,
        }))
    }
    pub(crate) fn complete(
        &mut self,
        lease: OriginLease,
        response: ExpressionResponse,
    ) -> Result<(), OriginError> {
        let receipt = self
            .receipts
            .get_mut(&lease.nonce)
            .ok_or_else(|| denied("expression/origin_expired"))?;
        if receipt.fingerprint != lease.fingerprint
            || Instant::now() >= receipt.expires
            || receipt.result.is_some()
        {
            return Err(denied("expression/origin_expired"));
        }
        receipt.result = Some(response);
        Ok(())
    }
}
