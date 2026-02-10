//! Integration tests for Phase 2: Identity and Security.
//!
//! These tests exercise cross-module interactions: identity ↔ trust,
//! identity ↔ auth handshake, trust persistence, and capability
//! grants with real cryptographic operations.

use rabbit_engine::protocol::frame::Frame;
use rabbit_engine::security::auth::{build_auth_proof, build_hello, Authenticator};
use rabbit_engine::security::identity::{fingerprint, format_burrow_id, parse_burrow_id, Identity};
use rabbit_engine::security::permissions::{Capability, CapabilityManager, Grant};
use rabbit_engine::security::trust::TrustCache;

use std::time::{Duration, Instant};

// ── Identity + Trust Integration ───────────────────────────────

#[test]
fn identity_save_load_same_burrow_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.key");

    let id1 = Identity::generate();
    let bid1 = id1.burrow_id();
    id1.save(&path).unwrap();

    let id2 = Identity::from_file(&path).unwrap();
    assert_eq!(id2.burrow_id(), bid1);
    // Signatures are compatible
    let data = b"test data for sign round trip";
    let sig = id1.sign(data);
    Identity::verify(&id2.public_key_bytes(), data, &sig).unwrap();
}

#[test]
fn trust_cache_save_load_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trusted_peers.tsv");

    let id1 = Identity::generate();
    let id2 = Identity::generate();

    let mut cache = TrustCache::new();
    cache
        .verify_or_remember(&id1.burrow_id(), &id1.public_key_bytes())
        .unwrap();
    cache
        .verify_or_remember(&id2.burrow_id(), &id2.public_key_bytes())
        .unwrap();
    cache.save(&path).unwrap();

    // Reload from disk
    let loaded = TrustCache::load(&path).unwrap();
    assert_eq!(loaded.len(), 2);

    // Both peers present with correct fingerprints
    let p1 = loaded.get(&id1.burrow_id()).unwrap();
    assert_eq!(p1.fingerprint, fingerprint(&id1.public_key_bytes()));
    let p2 = loaded.get(&id2.burrow_id()).unwrap();
    assert_eq!(p2.fingerprint, fingerprint(&id2.public_key_bytes()));
}

#[test]
fn trust_cache_load_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent.tsv");
    let cache = TrustCache::load(&path).unwrap();
    assert!(cache.is_empty());
}

#[test]
fn trust_rejects_changed_key_after_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trusted_peers.tsv");

    let id1 = Identity::generate();
    let impostor = Identity::generate();

    let mut cache = TrustCache::new();
    cache
        .verify_or_remember(&id1.burrow_id(), &id1.public_key_bytes())
        .unwrap();
    cache.save(&path).unwrap();

    // Reload and try with a different key for the same burrow ID
    let mut reloaded = TrustCache::load(&path).unwrap();
    let result = reloaded.verify_or_remember(&id1.burrow_id(), &impostor.public_key_bytes());
    assert!(result.is_err());
}

// ── Full Handshake Integration ─────────────────────────────────

#[test]
fn full_authenticated_handshake_with_trust() {
    let server = Identity::generate();
    let client = Identity::generate();
    let mut trust = TrustCache::new();

    // Server-side authenticator
    let mut auth = Authenticator::new(server, true);

    // Client builds HELLO
    let hello = build_hello(&client);
    let hello_wire = hello.serialize();
    let hello_parsed = Frame::parse(&hello_wire).unwrap();

    // Server processes HELLO → 300 CHALLENGE
    let challenge = auth.handle_hello(&hello_parsed).unwrap();
    assert_eq!(challenge.verb, "300");
    let challenge_wire = challenge.serialize();
    let challenge_parsed = Frame::parse(&challenge_wire).unwrap();

    // Client signs nonce → AUTH PROOF
    let proof = build_auth_proof(&client, &challenge_parsed).unwrap();
    let proof_wire = proof.serialize();
    let proof_parsed = Frame::parse(&proof_wire).unwrap();

    // Server verifies → 200 HELLO
    let response = auth.handle_auth(&proof_parsed).unwrap();
    assert_eq!(response.verb, "200");
    assert!(auth.is_authenticated());

    // After successful auth, server records peer in trust cache
    let peer_id = auth.peer_id().unwrap().to_string();
    let peer_pubkey = parse_burrow_id(&peer_id).unwrap();
    trust.verify_or_remember(&peer_id, &peer_pubkey).unwrap();
    assert_eq!(trust.len(), 1);
}

#[test]
fn anonymous_handshake_with_capabilities() {
    let server = Identity::generate();
    let mut auth = Authenticator::new(server, false);
    let mut caps = CapabilityManager::new();

    let hello = build_hello(&Identity::generate());
    let response = auth.handle_hello(&hello).unwrap();
    assert_eq!(response.verb, "200");
    assert_eq!(response.header("Burrow-ID"), Some("anonymous"));

    // Grant anonymous default capabilities
    let peer_id = auth.peer_id().unwrap();
    caps.grant(peer_id, Capability::Fetch, 3600);
    caps.grant(peer_id, Capability::List, 3600);

    assert!(caps.check("anonymous", Capability::Fetch));
    assert!(caps.check("anonymous", Capability::List));
    assert!(!caps.check("anonymous", Capability::Publish));
}

// ── Capability Expiry Integration ──────────────────────────────

#[test]
fn capability_expires_and_denied() {
    let mut mgr = CapabilityManager::new();

    // Grant with TTL that's already expired
    let expired_grant = Grant::with_created(
        Capability::Publish,
        Duration::from_millis(1),
        Instant::now() - Duration::from_secs(60),
    );
    mgr.grant_with("peer-a", expired_grant);

    // Should be denied
    assert!(!mgr.check("peer-a", Capability::Publish));

    // Grant a fresh one
    mgr.grant("peer-a", Capability::Publish, 3600);
    assert!(mgr.check("peer-a", Capability::Publish));
}

#[test]
fn multiple_peers_independent_capabilities() {
    let id1 = Identity::generate();
    let id2 = Identity::generate();
    let mut mgr = CapabilityManager::new();

    mgr.grant(&id1.burrow_id(), Capability::Publish, 3600);
    mgr.grant(&id2.burrow_id(), Capability::Subscribe, 3600);

    assert!(mgr.check(&id1.burrow_id(), Capability::Publish));
    assert!(!mgr.check(&id1.burrow_id(), Capability::Subscribe));
    assert!(!mgr.check(&id2.burrow_id(), Capability::Publish));
    assert!(mgr.check(&id2.burrow_id(), Capability::Subscribe));
}

// ── Identity ↔ Frame Integration ───────────────────────────────

#[test]
fn burrow_id_in_frame_round_trips() {
    let id = Identity::generate();
    let bid = id.burrow_id();

    let mut frame = Frame::new("200 HELLO");
    frame.set_header("Burrow-ID", &bid);
    let wire = frame.serialize();
    let parsed = Frame::parse(&wire).unwrap();

    let recovered_bid = parsed.header("Burrow-ID").unwrap();
    assert_eq!(recovered_bid, bid);

    // Parse the burrow ID back to pubkey bytes
    let pubkey = parse_burrow_id(recovered_bid).unwrap();
    assert_eq!(pubkey, id.public_key_bytes());

    // Formatted back matches
    assert_eq!(format_burrow_id(&pubkey), bid);
}

#[test]
fn signed_frame_body_verifiable() {
    let id = Identity::generate();
    let body = "This is an important announcement from the burrow.";

    // Sign the body
    let sig = id.sign(body.as_bytes());
    let sig_hex: String = sig.iter().map(|b| format!("{:02x}", b)).collect();

    // Put the signature in a frame header
    let mut frame = Frame::new("PUBLISH /q/announcements");
    frame.set_header("Lane", "3");
    frame.set_header("Sig", &format!("ed25519:{}", sig_hex));
    frame.set_body(body);

    let wire = frame.serialize();
    let parsed = Frame::parse(&wire).unwrap();

    // Verify the signature from the parsed frame
    let sig_header = parsed.header("Sig").unwrap();
    let sig_hex_parsed = sig_header.strip_prefix("ed25519:").unwrap();
    let sig_bytes: Vec<u8> = (0..sig_hex_parsed.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&sig_hex_parsed[i..i + 2], 16).unwrap())
        .collect();

    Identity::verify(
        &id.public_key_bytes(),
        parsed.body.as_deref().unwrap().as_bytes(),
        &sig_bytes,
    )
    .unwrap();
}

// ── Trust TSV Format Validation ────────────────────────────────

#[test]
fn trust_cache_tsv_is_human_readable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trusted_peers.tsv");

    let id = Identity::generate();
    let mut cache = TrustCache::new();
    cache
        .verify_or_remember(&id.burrow_id(), &id.public_key_bytes())
        .unwrap();
    cache.save(&path).unwrap();

    // Read the raw file and verify it's tab-separated text
    let content = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 1);

    let fields: Vec<&str> = lines[0].split('\t').collect();
    assert_eq!(fields.len(), 4);
    assert!(fields[0].starts_with("ed25519:"));
    assert_eq!(fields[1].len(), 64); // SHA-256 hex
    assert!(fields[2].parse::<u64>().is_ok()); // first_seen timestamp
    assert!(fields[3].parse::<u64>().is_ok()); // last_seen timestamp
}

#[test]
fn handshake_replay_protection() {
    // A captured AUTH proof should not work with a different nonce
    let server = Identity::generate();
    let client = Identity::generate();

    // First handshake
    let mut auth1 = Authenticator::new(server, true);
    let hello = build_hello(&client);
    let challenge1 = auth1.handle_hello(&hello).unwrap();
    let proof1 = build_auth_proof(&client, &challenge1).unwrap();
    auth1.handle_auth(&proof1).unwrap();

    // Second handshake with same server identity but new nonce
    let server2 = Identity::generate();
    let mut auth2 = Authenticator::new(server2, true);
    let hello2 = build_hello(&client);
    let _challenge2 = auth2.handle_hello(&hello2).unwrap();

    // Try to replay the proof from the first handshake
    // This should fail because the nonce is different
    let result = auth2.handle_auth(&proof1);
    assert!(result.is_err());
}
