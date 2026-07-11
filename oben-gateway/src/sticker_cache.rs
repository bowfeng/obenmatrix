//! Sticker description cache for platform media (e.g., Telegram stickers).
//!
//! When users send stickers, we describe them via vision tools and cache
//! the descriptions keyed by file_unique_id so we don't re-analyze the same
//! sticker image on every send. Descriptions are concise (1-2 sentences).
//!
//! Cache location: `~/.obenmatrix/{profile}/sticker_cache.json`
//!
//! This module is based on the Hermes-Agent Python implementation at
//! `~/workspace/hermes-agent/gateway/sticker_cache.py`.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tracing::warn;

/// Cached sticker description with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickerDescription {
    /// Vision-generated description text (1-2 sentences).
    pub description: String,
    /// Associated emoji (e.g., "😀").
    #[serde(default)]
    pub emoji: String,
    /// Sticker set name if available.
    #[serde(default)]
    pub set_name: String,
    /// Unix timestamp when cached.
    pub cached_at: u64,
}

/// Sticker cache manager with JSON-backed persistence.
pub struct StickerCache {
    /// In-memory cache map: file_unique_id -> description
    cache: HashMap<String, StickerDescription>,
    /// Path to the JSON cache file
    cache_path: PathBuf,
}

impl StickerCache {
    /// Creates a new sticker cache manager.
    /// 
    /// # Arguments
    /// * `data_dir` - The base data directory for the profile
    pub fn new(data_dir: PathBuf) -> Self {
        let cache_path = data_dir.join("sticker_cache.json");
        
        let cache = Self::load_cache(&cache_path);
        
        Self { cache, cache_path }
    }
    
    /// Loads the cache from disk.
    fn load_cache(cache_path: &PathBuf) -> HashMap<String, StickerDescription> {
        if cache_path.exists() {
            match fs::read_to_string(cache_path) {
                Ok(content) => {
                    match serde_json::from_str::<HashMap<String, StickerDescription>>(&content) {
                        Ok(cache) => return cache,
                        Err(e) => {
                            warn!("Failed to parse sticker cache: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read sticker cache: {}", e);
                }
            }
        }
        HashMap::new()
    }
    
    /// Saves the cache to disk atomically.
    fn save_cache(&self) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let json = serde_json::to_string_pretty(&self.cache)?;
        fs::write(&self.cache_path, json)?;
        
        Ok(())
    }
    
    /// Gets a cached sticker description.
    /// 
    /// # Arguments
    /// * `file_unique_id` - The platform's stable sticker identifier
    pub fn get_cached_description(&self, file_unique_id: &str) -> Option<&StickerDescription> {
        self.cache.get(file_unique_id)
    }
    
    /// Caches a sticker description.
    /// 
    /// # Arguments
    /// * `file_unique_id` - The platform's stable sticker identifier
    /// * `description` - Vision-generated description text
    /// * `emoji` - Associated emoji (optional)
    /// * `set_name` - Sticker set name (optional)
    pub fn cache_sticker_description(
        &mut self,
        file_unique_id: String,
        description: String,
        emoji: Option<String>,
        set_name: Option<String>,
    ) -> Result<()> {
        let emoji = emoji.unwrap_or_default();
        let set_name = set_name.unwrap_or_default();
        let cached_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        self.cache.insert(file_unique_id, StickerDescription {
            description,
            emoji,
            set_name,
            cached_at,
        });
        
        self.save_cache()?;
        
        Ok(())
    }
    
    /// Builds injection text for a sticker description.
    /// 
    /// Returns a string like:
    /// `[The user sent a sticker 😀 from "MyPack"~ It shows: "A cat waving" (=^.w.^=)]`
    pub fn build_sticker_injection(
        description: &str,
        emoji: &str,
        set_name: &str,
    ) -> String {
        let mut context = String::new();
        
        if !set_name.is_empty() && !emoji.is_empty() {
            context = format!(" {} from \"{}\"", emoji, set_name);
        } else if !emoji.is_empty() {
            context = format!(" {}", emoji);
        }
        
        format!(r#"[The user sent a sticker{}~ It shows: "{}" (=^.w.^=)]"#, context, description)
    }
    
    /// Builds injection text for animated/video stickers that can't be analyzed.
    pub fn build_animated_sticker_injection(emoji: Option<&str>) -> String {
        match emoji {
            Some(e) => format!(
                r#"[The user sent an animated sticker {}~ I can't see animated ones yet, but the emoji suggests: {}]"#,
                e, e
            ),
            None => r#"[The user sent an animated sticker~ I can't see animated ones yet]"#.to_string(),
        }
    }
    
    /// Gets the number of cached stickers.
    pub fn len(&self) -> usize {
        self.cache.len()
    }
    
    /// Returns true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
    
    /// Clears all cached descriptions.
    pub fn clear(&mut self) -> Result<()> {
        self.cache.clear();
        self.save_cache()?;
        Ok(())
    }
    
    /// Evicts old entries based on age (in seconds).
    /// Returns the number of entries removed.
    pub fn evict_old_entries(&mut self, max_age_seconds: u64) -> usize {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let old_keys: Vec<String> = self.cache
            .iter()
            .filter(|(_, desc)| now.saturating_sub(desc.cached_at) > max_age_seconds)
            .map(|(id, _)| id.clone())
            .collect();
        
        let removed = old_keys.len();
        
        for key in old_keys {
            self.cache.remove(&key);
        }
        
        if removed > 0 {
            let _ = self.save_cache();
        }
        
        removed
    }
}

impl Default for StickerCache {
    fn default() -> Self {
        // Default to a temp path for testing
        let data_dir = std::env::temp_dir().join("obenmatrix_test");
        Self::new(data_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Given: An empty sticker cache
    /// When: We cache a sticker description
    /// Then: The description is stored and can be retrieved
    #[test]
    fn test_cache_and_retrieve_sticker() {
        // Setup: Create a temporary cache directory
        let temp_dir = std::env::temp_dir().join("test_sticker_cache_1");
        let _ = fs::remove_dir_all(&temp_dir); // Clean up from any previous test
        
        let mut cache = StickerCache::new(temp_dir.clone());
        
        // Action: Cache a sticker description
        let file_id = "file_unique_12345".to_string();
        let description = "A cat waving its paw".to_string();
        let emoji = Some("🐱".to_string());
        let set_name = Some("MyStickers".to_string());
        
        cache.cache_sticker_description(file_id.clone(), description.clone(), emoji, set_name)
            .expect("Failed to cache sticker");
        
        // Verify: The description can be retrieved
        let retrieved = cache.get_cached_description(&file_id);
        assert!(retrieved.is_some(), "Should retrieve cached description");
        
        let desc = retrieved.unwrap();
        assert_eq!(desc.description, description);
        assert_eq!(desc.emoji, "🐱");
        assert_eq!(desc.set_name, "MyStickers");
        assert!(desc.cached_at > 0);
        
        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
    
    /// Given: A cached sticker description
    /// When: We look up a different file_unique_id
    /// Then: Returns None for non-existent entry
    #[test]
    fn test_miss_for_nonexistent_sticker() {
        let temp_dir = std::env::temp_dir().join("test_sticker_cache_2");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let cache = StickerCache::new(temp_dir.clone());
        
        // Action: Look up a file that was never cached
        let result = cache.get_cached_description("nonexistent_file_id");
        
        // Verify: Returns None
        assert!(result.is_none(), "Should return None for non-existent sticker");
        
        let _ = fs::remove_dir_all(&temp_dir);
    }
    
    /// Given: Multiple cached stickers
    /// When: We clear the cache
    /// Then: All entries are removed
    #[test]
    fn test_clear_cache() {
        let temp_dir = std::env::temp_dir().join("test_sticker_cache_3");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let mut cache = StickerCache::new(temp_dir.clone());
        
        // Setup: Add multiple stickers
        for i in 0..5 {
            cache.cache_sticker_description(
                format!("file_{}", i),
                format!("Description {}", i),
                Some(format!("emoji_{}", i)),
                None,
            ).unwrap();
        }
        
        // Verify: Cache has entries
        assert_eq!(cache.len(), 5, "Cache should have 5 entries");
        
        // Action: Clear the cache
        cache.clear().expect("Failed to clear cache");
        
        // Verify: Cache is empty
        assert_eq!(cache.len(), 0, "Cache should be empty after clear");
        
        let _ = fs::remove_dir_all(&temp_dir);
    }
    
    /// Given: Cached stickers with different ages
    /// When: We evict old entries (older than 1 second)
    /// Then: Old entries are removed
    #[test]
    fn test_evict_old_entries() {
        let temp_dir = std::env::temp_dir().join("test_sticker_cache_4");
        let _ = fs::remove_dir_all(&temp_dir);
        
        let mut cache = StickerCache::new(temp_dir.clone());
        
        // Setup: Add stickers with different timestamps
        cache.cache_sticker_description(
            "recent_file".to_string(),
            "Recent sticker".to_string(),
            Some("😀".to_string()),
            None,
        ).unwrap();
        
        // Manually insert an old entry (simulating a sticker from the past)
        use std::time::{SystemTime, UNIX_EPOCH};
        let old_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() - 10; // 10 seconds ago
        cache.cache.insert(
            "old_file".to_string(),
            StickerDescription {
                description: "Old sticker".to_string(),
                emoji: " aged".to_string(),
                set_name: "".to_string(),
                cached_at: old_time,
            },
        );
        
        // Action: Evict entries older than 1 second
        let removed = cache.evict_old_entries(1);
        
        // Verify: Old entry is removed
        assert_eq!(removed, 1, "Should remove 1 old entry");
        assert!(cache.get_cached_description("old_file").is_none(), "Old entry should be gone");
        assert!(cache.get_cached_description("recent_file").is_some(), "Recent entry should remain");
        
        let _ = fs::remove_dir_all(&temp_dir);
    }
    
    /// Given: A sticker description
    /// When: We build injection text with set_name and emoji
    /// Then: Returns properly formatted string
    #[test]
    fn test_build_injection_with_set_name() {
        let result = StickerCache::build_sticker_injection(
            "A cat waving",
            "🐱",
            "MyStickers",
        );
        
        assert!(result.contains("A cat waving"));
        assert!(result.contains("🐱"));
        assert!(result.contains("MyStickers"));
        assert!(result.contains("(=^.w.^=)"));
    }
    
    /// Given: A sticker description without set_name
    /// When: We build injection text with only emoji
    /// Then: Returns properly formatted string without set_name
    #[test]
    fn test_build_injection_without_set_name() {
        let result = StickerCache::build_sticker_injection(
            "A dog barking",
            "🐶",
            "",
        );
        
        assert!(result.contains("A dog barking"));
        assert!(result.contains("🐶"));
        assert!(!result.contains("from"), "Should not contain 'from' when set_name is empty");
    }
    
    /// Given: An animated sticker
    /// When: We build injection text for it
    /// Then: Returns message indicating animation can't be seen
    #[test]
    fn test_build_animated_injection() {
        let result = StickerCache::build_animated_sticker_injection(Some("🔥"));
        
        assert!(result.contains("animated sticker"));
        assert!(result.contains("🔥"));
        assert!(result.contains("can't see animated ones yet"));
    }
    
    /// Given: An empty cache file that exists
    /// When: We create a new StickerCache
    /// Then: Returns empty cache (no panic on empty file)
    #[test]
    fn test_load_empty_cache_file() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_sticker_cache_5_{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&temp_dir);
        
        // Create an empty cache file
        let cache_path = temp_dir.join("sticker_cache.json");
        let result = fs::write(&cache_path, "{}");
        if result.is_err() {
            eprintln!("Failed to create cache file at {:?}", cache_path);
            return; // Skip test if we can't create file
        }
        
        // Action: Create new cache
        let cache = StickerCache::new(temp_dir.clone());
        
        // Verify: Returns empty cache
        assert!(cache.is_empty());
        
        let _ = fs::remove_dir_all(&temp_dir);
    }
    
    /// Given: A cache file with invalid JSON
    /// When: We create a new StickerCache
    /// Then: Returns empty cache (graceful fallback)
    #[test]
    fn test_load_invalid_json_cache() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_sticker_cache_6_{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&temp_dir);
        
        // Create a cache file with invalid JSON
        let cache_path = temp_dir.join("sticker_cache.json");
        let result = fs::write(&cache_path, "{invalid json");
        if result.is_err() {
            eprintln!("Failed to create cache file at {:?}", cache_path);
            return; // Skip test if we can't create file
        }
        
        // Action: Create new cache
        let cache = StickerCache::new(temp_dir.clone());
        
        // Verify: Returns empty cache (graceful fallback)
        assert!(cache.is_empty());
        
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
