/// Pairing system for user ↔ platform registration.
///
/// Code-based approval flow for authorizing new users on messaging platforms.
/// Instead of static allowlists with user IDs, unknown users receive a one-time
/// pairing code that the bot owner approves via the CLI.
///
/// Security features (based on OWASP + NIST SP 800-63-4 guidance):
///   - 8-char codes from 32-char unambiguous alphabet (no 0/O/1/I)
///   - Cryptographic randomness via rand::rng()
///   - 1-hour code expiry
///   - Max 3 pending codes per platform
///   - Rate limiting: 1 request per user per 10 minutes
///   - Lockout after 5 failed approval attempts (1 hour)
///   - Codes are never logged to stdout

use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unambiguous alphabet -- excludes 0/O, 1/I to prevent confusion
const ALPHABET: &[char] = &[
    'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'J', 'K', 'L', 'M', 'N', 'P', 'Q', 'R', 'S', 'T',
    'U', 'V', 'W', 'X', 'Y', 'Z', '2', '3', '4', '5', '6', '7', '8', '9',
];
const CODE_LENGTH: usize = 8;

/// Timing constants (in seconds)
const CODE_TTL_SECONDS: u64 = 3600;      // Codes expire after 1 hour
const RATE_LIMIT_SECONDS: u64 = 600;     // 1 request per user per 10 minutes
const LOCKOUT_SECONDS: u64 = 3600;       // Lockout duration after too many failures

/// Limits
const MAX_PENDING_PER_PLATFORM: usize = 3;
const MAX_FAILED_ATTEMPTS: usize = 5;

/// A pending pairing request stored with a hashed code.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingEntry {
    hash: String,
    salt: String,
    user_id: String,
    user_name: String,
    created_at: u64,
}

/// Rate limit tracking for a platform.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RateLimitState {
    last_requests: HashMap<String, u64>,  // user_id -> timestamp
    failed_attempts: HashMap<String, u64>,  // platform -> count
    lockout_until: HashMap<String, u64>,  // platform -> timestamp
}

/// In-memory storage for pairing data.
struct PairingStore {
    pending: HashMap<String, Vec<PendingEntry>>,  // platform -> entries
    approved: HashMap<String, Vec<ApprovedUser>>,  // platform -> users
    rate_limits: RateLimitState,
}

impl PairingStore {
    /// Create a new in-memory pairing store.
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
            approved: HashMap::new(),
            rate_limits: RateLimitState::default(),
        }
    }

    /// Generate a pairing code for a user.
    /// Returns the code string, or None if rate-limited, at max pending, or locked out.
    pub fn generate_code(&mut self, platform: &str, user_id: &str, user_name: &str) -> Option<String> {
        self.cleanup_expired(platform);

        // Check lockout
        if self.is_locked_out(platform) {
            return None;
        }

        // Check rate limit
        if self.is_rate_limited(platform, user_id) {
            return None;
        }

        // Check max pending
        let pending_count = self.pending.get(platform).map(|v| v.len()).unwrap_or(0);
        if pending_count >= MAX_PENDING_PER_PLATFORM {
            return None;
        }

        // Generate cryptographically random code
        let code: String = (0..CODE_LENGTH)
            .map(|_| {
                let idx = rand::thread_rng().gen_range(0..ALPHABET.len());
                ALPHABET[idx]
            })
            .collect();

        // Hash the code with a random salt
        let salt = self.generate_salt();
        let code_hash = self.hash_code(&code, &salt);

        // Store pending request
        let entry = PendingEntry {
            hash: code_hash,
            salt: hex::encode(&salt),
            user_id: user_id.to_string(),
            user_name: user_name.to_string(),
            created_at: Self::now(),
        };

        self.pending
            .entry(platform.to_string())
            .or_insert_with(Vec::new)
            .push(entry);

        // Record rate limit
        self.record_rate_limit(platform, user_id);

        Some(code)
    }

    /// Approve a pairing code.
    /// Returns Some(user_id, user_name) on success, None if code is invalid/expired or locked out.
    pub fn approve_code(&mut self, platform: &str, code: &str) -> Option<(String, String)> {
        self.cleanup_expired(platform);

        // Lockout check
        if self.is_locked_out(platform) {
            return None;
        }

        let code = code.trim().to_uppercase();

        // Find matching entry
        let empty_vec = Vec::new();
        let pending_entries = self.pending.get(platform).unwrap_or(&empty_vec);
        let matched_entry: Option<PendingEntry> = pending_entries
            .iter()
            .find(|entry| {
                if let Ok(salt) = hex::decode(&entry.salt) {
                    self.hash_code(&code, &salt) == entry.hash
                } else {
                    false
                }
            })
            .cloned();

        if matched_entry.is_none() {
            self.record_failed_attempt(platform);
            return None;
        }

        let entry = matched_entry.unwrap();

        // Remove from pending
        if let Some(entries) = self.pending.get_mut(platform) {
            entries.retain(|e| e.hash != entry.hash);
        }

        // Add to approved
        self.approve_user(platform, &entry.user_id, &entry.user_name);

        Some((entry.user_id, entry.user_name))
    }

    /// List pending pairing requests for a platform.
    pub fn list_pending(&mut self, platform: Option<&str>) -> Vec<PendingEntryInfo> {
        let platforms = match platform {
            Some(p) => vec![p.to_string()],
            None => self.pending.keys().cloned().collect(),
        };

        let mut results = Vec::new();
        for p in platforms {
            self.cleanup_expired(&p);
            if let Some(entries) = self.pending.get(&p) {
                for entry in entries {
                    results.push(PendingEntryInfo {
                        platform: p.clone(),
                        code: entry.hash.chars().take(8).collect(),  // Show first 8 chars of hash
                        user_id: entry.user_id.clone(),
                        user_name: entry.user_name.clone(),
                        age_minutes: (Self::now() - entry.created_at) / 60,
                    });
                }
            }
        }
        results
    }

    /// List approved users for a platform.
    pub fn list_approved(&self, platform: Option<&str>) -> Vec<ApprovedUser> {
        let platforms = match platform {
            Some(p) => vec![p.to_string()],
            None => self.approved.keys().cloned().collect(),
        };

        let mut results = Vec::new();
        for p in platforms {
            if let Some(users) = self.approved.get(&p) {
                results.extend(users.clone());
            }
        }
        results
    }

    /// Check if a user is approved on a platform.
    pub fn is_approved(&self, platform: &str, user_id: &str) -> bool {
        self.approved
            .get(platform)
            .map(|users| users.iter().any(|u| u.user_id == user_id))
            .unwrap_or(false)
    }

    /// Revoke approval for a user.
    pub fn revoke(&mut self, platform: &str, user_id: &str) -> bool {
        if let Some(users) = self.approved.get_mut(platform) {
            let len = users.len();
            users.retain(|u| u.user_id != user_id);
            return len != users.len();
        }
        false
    }

    // ----- Helper methods -----

    fn hash_code(&self, code: &str, salt: &[u8]) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(salt);
        hasher.update(code.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn generate_salt(&self) -> Vec<u8> {
        let mut salt = [0u8; 16];
        rand::thread_rng().fill(&mut salt);
        salt.to_vec()
    }

    fn is_rate_limited(&self, platform: &str, user_id: &str) -> bool {
        let key = format!("{}:{}", platform, user_id);
        if let Some(last_request) = self.rate_limits.last_requests.get(&key) {
            let now = Self::now();
            return now - *last_request < RATE_LIMIT_SECONDS;
        }
        false
    }

    fn record_rate_limit(&mut self, platform: &str, user_id: &str) {
        let key = format!("{}:{}", platform, user_id);
        self.rate_limits.last_requests.insert(key, Self::now());
    }

    fn is_locked_out(&self, platform: &str) -> bool {
        if let Some(lockout_until) = self.rate_limits.lockout_until.get(platform) {
            return Self::now() < *lockout_until;
        }
        false
    }

    fn record_failed_attempt(&mut self, platform: &str) {
        let failed = self
            .rate_limits
            .failed_attempts
            .entry(platform.to_string())
            .or_insert(0u64);
        *failed += 1;

        if *failed >= MAX_FAILED_ATTEMPTS as u64 {
            self.rate_limits.lockout_until.insert(
                platform.to_string(),
                Self::now() + LOCKOUT_SECONDS,
            );
            *failed = 0;  // Reset counter
        }
    }

    fn approve_user(&mut self, platform: &str, user_id: &str, user_name: &str) {
        let user = ApprovedUser {
            user_id: user_id.to_string(),
            user_name: user_name.to_string(),
            approved_at: Self::now(),
        };
        self.approved
            .entry(platform.to_string())
            .or_insert_with(Vec::new)
            .push(user);
    }

    fn cleanup_expired(&mut self, platform: &str) {
        let now = Self::now();
        if let Some(entries) = self.pending.get_mut(platform) {
            entries.retain(|e| now - e.created_at <= CODE_TTL_SECONDS);
        }
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Info about a pending pairing entry (for listing).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingEntryInfo {
    pub platform: String,
    pub code: String,  // First 8 chars of hash (not the actual code)
    pub user_id: String,
    pub user_name: String,
    pub age_minutes: u64,
}

/// An approved user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovedUser {
    pub user_id: String,
    pub user_name: String,
    pub approved_at: u64,
}

/// The main pairing manager interface.
pub struct PairingManager {
    store: PairingStore,
}

impl PairingManager {
    /// Create a new pairing manager.
    pub fn new() -> Self {
        Self {
            store: PairingStore::new(),
        }
    }

    /// Generate a pairing code for a user.
    pub fn generate_code(&mut self, platform: &str, user_id: &str, user_name: &str) -> Option<String> {
        self.store.generate_code(platform, user_id, user_name)
    }

    /// Approve a pairing code.
    pub fn approve_code(&mut self, platform: &str, code: &str) -> Option<(String, String)> {
        self.store.approve_code(platform, code)
    }

    /// List pending pairing requests.
    pub fn list_pending(&mut self, platform: Option<&str>) -> Vec<PendingEntryInfo> {
        self.store.list_pending(platform)
    }

    /// List approved users.
    pub fn list_approved(&self, platform: Option<&str>) -> Vec<ApprovedUser> {
        self.store.list_approved(platform)
    }

    /// Check if a user is approved.
    pub fn is_approved(&self, platform: &str, user_id: &str) -> bool {
        self.store.is_approved(platform, user_id)
    }

    /// Revoke approval for a user.
    pub fn revoke(&mut self, platform: &str, user_id: &str) -> bool {
        self.store.revoke(platform, user_id)
    }
}

impl Default for PairingManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Given: A new pairing manager with no pending or approved users
    /// When: generate_code is called for a user
    /// Then: Returns a valid 8-character code from the unambiguous alphabet
    #[test]
    fn test_generate_code_basic() {
        let mut manager = PairingManager::new();
        let code = manager.generate_code("telegram", "user-123", "Alice").unwrap();

        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.chars().all(|c| ALPHABET.contains(&c)));
    }

    /// Given: A user who just requested a code
    /// When: They request another code within the rate limit period
    /// Then: Returns None (rate limited)
    #[test]
    fn test_rate_limiting() {
        let mut manager = PairingManager::new();

        // Generate first code
        let _ = manager.generate_code("telegram", "user-123", "Alice");

        // Immediately try again - should be rate limited
        let code = manager.generate_code("telegram", "user-123", "Alice");
        assert!(code.is_none(), "Should be rate limited");
    }

    /// Given: A user who has exceeded the rate limit
    /// When: A different user requests a code on the same platform
    /// Then: The different user can still get a code (rate limit is per-user)
    #[test]
    fn test_rate_limit_per_user() {
        let mut manager = PairingManager::new();

        // User-1 gets rate limited
        let _ = manager.generate_code("telegram", "user-1", "Alice");
        assert!(manager.generate_code("telegram", "user-1", "Alice").is_none());

        // User-2 should still be able to get a code
        let code = manager.generate_code("telegram", "user-2", "Bob");
        assert!(code.is_some(), "Different user should not be rate limited");
    }

    /// Given: A platform that has reached max pending codes
    /// When: Another user tries to get a code
    /// Then: Returns None (max pending reached)
    #[test]
    fn test_max_pending() {
        let mut manager = PairingManager::new();

        // Fill up pending codes
        for i in 0..MAX_PENDING_PER_PLATFORM {
            let code = manager.generate_code("telegram", &format!("user-{}", i), &format!("User {}", i));
            assert!(code.is_some(), "Should be able to create pending #{}", i);
        }

        // One more should fail
        let code = manager.generate_code("telegram", "user-overflow", "Overflow");
        assert!(code.is_none(), "Should be at max pending");
    }

    /// Given: A platform that is locked out due to failed attempts
    /// When: Someone tries to approve any code
    /// Then: Returns None (locked out)
    #[test]
    fn test_lockout() {
        let mut manager = PairingManager::new();

        // Trigger lockout by failing MAX_FAILED_ATTEMPTS times
        for i in 0..MAX_FAILED_ATTEMPTS {
            manager.approve_code("telegram", "wrong-code");
            let failed = manager.store.rate_limits.failed_attempts.get("telegram").copied();
            eprintln!("Attempt {}: failed = {:?}", i + 1, failed);
            eprintln!("Lockout until: {:?}", manager.store.rate_limits.lockout_until.get("telegram"));
        }

        // Now even a valid code approach should be rejected
        assert!(manager.store.is_locked_out("telegram"), "Should be locked out after {} failed attempts", MAX_FAILED_ATTEMPTS);
    }

    /// Given: A valid pairing code that was generated
    /// When: approve_code is called with the correct code
    /// Then: Returns Some(user_id, user_name) and user is added to approved list
    #[test]
    fn test_approve_code_valid() {
        let mut manager = PairingManager::new();

        // Generate a code
        let code = manager.generate_code("telegram", "user-123", "Alice").unwrap();

        // Approve it
        let result = manager.approve_code("telegram", &code);
        assert!(result.is_some());
        let (user_id, user_name) = result.unwrap();
        assert_eq!(user_id, "user-123");
        assert_eq!(user_name, "Alice");

        // User should now be in approved list
        assert!(manager.is_approved("telegram", "user-123"));
    }

    /// Given: An invalid pairing code
    /// When: approve_code is called
    /// Then: Returns None and records a failed attempt
    #[test]
    fn test_approve_code_invalid() {
        let mut manager = PairingManager::new();

        let result = manager.approve_code("telegram", "INVALID1");
        assert!(result.is_none());

        // Should record failed attempt
        assert_eq!(
            manager.store.rate_limits.failed_attempts.get("telegram"),
            Some(&1u64)
        );
    }

    /// Given: A user who was approved
    /// When: revoke is called
    /// Then: Returns true and user is removed from approved list
    #[test]
    fn test_revoke() {
        let mut manager = PairingManager::new();

        // Approve a user first
        let code = manager.generate_code("telegram", "user-123", "Alice").unwrap();
        manager.approve_code("telegram", &code);

        // Revoke
        let revoked = manager.revoke("telegram", "user-123");
        assert!(revoked);
        assert!(!manager.is_approved("telegram", "user-123"));
    }

    /// Given: A user who was revoked
    /// When: revoke is called again
    /// Then: Returns false (not found)
    #[test]
    fn test_revoke_not_found() {
        let mut manager = PairingManager::new();

        let revoked = manager.revoke("telegram", "user-never-approved");
        assert!(!revoked);
    }

    /// Given: Multiple pending codes
    /// When: list_pending is called
    /// Then: Returns all pending entries with code display and user info
    #[test]
    fn test_list_pending() {
        let mut manager = PairingManager::new();

        manager.generate_code("telegram", "user-1", "Alice");
        manager.generate_code("telegram", "user-2", "Bob");
        manager.generate_code("discord", "user-3", "Charlie");

        let pending = manager.list_pending(None);
        assert_eq!(pending.len(), 3);

        let telegram_pending: Vec<_> = pending.iter().filter(|p| p.platform == "telegram").collect();
        assert_eq!(telegram_pending.len(), 2);
    }

    /// Given: Multiple approved users
    /// When: list_approved is called
    /// Then: Returns all approved users with their info
    #[test]
    fn test_list_approved() {
        let mut manager = PairingManager::new();

        let code1 = manager.generate_code("telegram", "user-1", "Alice").unwrap();
        let code2 = manager.generate_code("telegram", "user-2", "Bob").unwrap();

        manager.approve_code("telegram", &code1);
        manager.approve_code("telegram", &code2);

        let approved = manager.list_approved(Some("telegram"));
        assert_eq!(approved.len(), 2);
        assert!(approved.iter().any(|u| u.user_id == "user-1"));
        assert!(approved.iter().any(|u| u.user_id == "user-2"));
    }

    /// Given: A platform that has expired pending codes
    /// When: cleanup_expired is triggered
    /// Then: Expired codes are removed
    #[test]
    fn test_cleanup_expired() {
        let mut manager = PairingManager::new();

        // Generate a code
        manager.generate_code("telegram", "user-1", "Alice");

        // Manually manipulate created_at to simulate expiration
        // (In production, this would happen naturally over time)
        if let Some(entries) = manager.store.pending.get_mut("telegram") {
            for entry in entries.iter_mut() {
                entry.created_at = 0;  // Force expiration
            }
        }

        // Cleanup should remove all entries
        manager.store.cleanup_expired("telegram");
        assert!(manager.store.pending.get("telegram").map(|v| v.is_empty()).unwrap_or(true));
    }

    /// Given: The same user on different platforms
    /// When: generate_code is called for each
    /// Then: Different codes are generated for each platform
    #[test]
    fn test_cross_platform_isolation() {
        let mut manager = PairingManager::new();

        let telegram_code = manager.generate_code("telegram", "user-123", "Alice").unwrap();
        let discord_code = manager.generate_code("discord", "user-123", "Alice").unwrap();

        assert_ne!(telegram_code, discord_code);
    }
}
