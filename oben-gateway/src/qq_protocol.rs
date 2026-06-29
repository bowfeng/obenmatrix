//! QQ Bot WebSocket Gateway protocol types.
//!
//! Defines all message types, opcodes, intent bitflags, and close codes used
//! by the QQ Open Platform WebSocket Gateway (wss://gw.open.q.qq.com).
//!
//! Reference: https://bot.q.qq.com/wiki/develop/api-v2/

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Opcodes
// ---------------------------------------------------------------------------

/// WebSocket gateway opcodes (bidirectional).
/// Manual serde impl to avoid `serde_repr` as a dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    Dispatch = 0,
    Heartbeat = 1,
    Identify = 2,
    Resume = 6,
    Reconnect = 7,
    InvalidSession = 9,
    Hello = 10,
    HeartbeatAck = 11,
    HttpCallbackAck = 12,
    UrlValidation = 13,
}

impl serde::Serialize for OpCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_i32(*self as i32)
    }
}

impl<'de> serde::Deserialize<'de> for OpCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = i32::deserialize(deserializer)?;
        Ok(match value {
            0 => Self::Dispatch,
            1 => Self::Heartbeat,
            2 => Self::Identify,
            6 => Self::Resume,
            7 => Self::Reconnect,
            9 => Self::InvalidSession,
            10 => Self::Hello,
            11 => Self::HeartbeatAck,
            12 => Self::HttpCallbackAck,
            13 => Self::UrlValidation,
            v => panic!("Unknown OpCode variant: {v}"),
        })
    }
}

// ---------------------------------------------------------------------------
// Incoming server messages (op:0 dispatch, op:10 hello, op:1 heartbeat, …)
// ---------------------------------------------------------------------------

/// Top-level WebSocket frame from the server.
#[derive(Debug, Clone, Deserialize)]
pub struct WsIncomingMessage {
    pub op: OpCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<String>,
    pub d: serde_json::Value,
}

/// Server `op:10 Hello` payload — contains heartbeat interval.
#[derive(Debug, Clone, Deserialize)]
pub struct HelloPayload {
    pub heartbeat_interval: u64, // milliseconds
}

/// Server `op:0 READY` event — confirms identification succeeded.
#[derive(Debug, Clone, Deserialize)]
pub struct ReadyPayload {
    pub version: i32,
    pub session_id: String,
    #[serde(default)]
    pub user: ReadyUser,
    #[serde(default)]
    pub shard: Option<[i64; 2]>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ReadyUser {
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub bot: bool,
}

/// Server `op:7 Reconnect` — tells client to reconnect with resume.
#[derive(Debug, Clone, Deserialize)]
pub struct ReconnectPayload {} // no extra data

/// Server `op:0 RESUMED` — server resumed missing events.
#[derive(Debug, Clone, Deserialize)]
pub struct ResumedPayload(pub String);

// ---------------------------------------------------------------------------
// Client-to-server messages
// ---------------------------------------------------------------------------

/// Identify payload (op:2) — authenticates the bot.
#[derive(Debug, Clone, Serialize)]
pub struct IdentifyPayload {
    pub token: String,
    pub intents: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard: Option<[usize; 2]>,
    #[serde(rename = "properties")]
    pub properties: Properties,
}

#[derive(Debug, Clone, Serialize)]
pub struct Properties {
    #[serde(rename = "$os")]
    pub os: &'static str,
    #[serde(rename = "$browser")]
    pub browser: &'static str,
    #[serde(rename = "$device")]
    pub device: &'static str,
}

impl Default for Properties {
    fn default() -> Self {
        Self {
            os: "linux",
            browser: "bot",
            device: "bot",
        }
    }
}

/// Resume payload (op:6) — reconnects with last known position.
#[derive(Debug, Clone, Serialize)]
pub struct ResumePayload {
    pub token: String,
    pub session_id: String,
    pub seq: i64,
}

/// Heartbeat payload — sends last received `s` sequence number.
#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatPayload(#[serde(skip_serializing_if = "Option::is_none")] pub Option<i64>);

// ---------------------------------------------------------------------------
// Close codes
// ---------------------------------------------------------------------------

/// QQ Bot WebSocket close reason codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseCode {
    Unknown(u16),
    /// Invalid opcode — don't reconnect.
    InvalidOpcode,
    /// Invalid payload — don't reconnect.
    InvalidPayload,
    /// Invalid session — re-identify.
    InvalidSession,
    /// Invalid seq — re-identify.
    InvalidSeq,
    /// Rate limited — backoff then reconnect.
    RateLimited,
    /// Connection expired — resume.
    ConnectionExpired,
    /// Invalid shard — don't reconnect.
    InvalidShard,
    /// Too many guilds/shards — don't reconnect.
    TooManyGuilds,
    /// Invalid version — don't reconnect.
    InvalidVersion,
    /// Invalid intent — don't reconnect.
    InvalidIntent,
    /// Intent permission denied — don't reconnect.
    IntentDenied,
    /// Invalid token — refresh token + re-identify.
    InvalidToken,
    /// Sandbox-only bot — stop forever.
    SandboxOnly,
    /// Banned — stop forever.
    Banned,
}

impl CloseCode {
    pub fn from_code(code: u16) -> Self {
        match code {
            4001 => Self::InvalidOpcode,
            4002 => Self::InvalidPayload,
            4006 => Self::InvalidSession,
            4007 => Self::InvalidSeq,
            4008 => Self::RateLimited,
            4009 => Self::ConnectionExpired,
            4010 => Self::InvalidShard,
            4011 => Self::TooManyGuilds,
            4012 => Self::InvalidVersion,
            4013 => Self::InvalidIntent,
            4014 => Self::IntentDenied,
            4004 => Self::InvalidToken,
            4914 => Self::SandboxOnly,
            4915 => Self::Banned,
            _ => Self::Unknown(code),
        }
    }

    /// Whether the bot should retry with re-identify (not resume).
    pub fn requires_reidentify(&self) -> bool {
        matches!(
            self,
            Self::InvalidSession | Self::InvalidSeq | Self::InvalidToken
        )
    }

    /// Whether the bot should resume (with session + seq).
    pub fn should_resume(&self) -> bool {
        matches!(
            self,
            Self::ConnectionExpired | Self::RateLimited
        )
    }

    /// Fatal codes — never retry.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::InvalidOpcode
                | Self::InvalidPayload
                | Self::InvalidShard
                | Self::TooManyGuilds
                | Self::InvalidVersion
                | Self::InvalidIntent
                | Self::IntentDenied
                | Self::SandboxOnly
                | Self::Banned
        )
    }
}

// ---------------------------------------------------------------------------
// Intent bitflags
// ---------------------------------------------------------------------------

/// QQ Bot intent bitflags — OR these together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Intents(u64);

impl Intents {
    /// Guild events: GUILD_CREATE/UPDATE/DELETE, CHANNEL_CREATE/UPDATE/DELETE
    pub const GUILDS: u64 = 1 << 0;
    /// GUILD_MEMBER_ADD/UPDATE/REMOVE
    pub const GUILD_MEMBERS: u64 = 1 << 1;
    /// GUILD_MESSAGES (private-domain bots): MESSAGE_CREATE/DELETE
    pub const GUILD_MESSAGES: u64 = 1 << 9;
    /// MESSAGE_REACTION_ADD/REMOVE
    pub const GUILD_MESSAGE_REACTIONS: u64 = 1 << 10;
    /// DIRECT_MESSAGE_CREATE/DELETE
    pub const DIRECT_MESSAGE: u64 = 1 << 12;
    /// C2C/GROUP/C2C_MSG_RECEIVE/C2C_MSG_REJECT/GROUP_*_ROBOT/***_MSG_RECEIVE
    pub const GROUP_AND_C2C: u64 = 1 << 25;
    /// INTERACTION_CREATE
    pub const INTERACTION: u64 = 1 << 26;
    /// MESSAGE_AUDIT_PASS/REJECT
    pub const MESSAGE_AUDIT: u64 = 1 << 27;
    /// AUDIO_START/FINISH/ON_MIC/OFF_MIC
    pub const AUDIO_ACTION: u64 = 1 << 30;

    pub fn new() -> Self {
        Self(0)
    }

    pub fn with_guilds(mut self) -> Self {
        self.0 |= Self::GUILDS;
        self
    }

    pub fn with_guild_members(mut self) -> Self {
        self.0 |= Self::GUILD_MEMBERS;
        self
    }

    pub fn with_group_and_c2c(mut self) -> Self {
        self.0 |= Self::GROUP_AND_C2C;
        self
    }

    pub fn with_direct_message(mut self) -> Self {
        self.0 |= Self::DIRECT_MESSAGE;
        self
    }

    pub fn with_interaction(mut self) -> Self {
        self.0 |= Self::INTERACTION;
        self
    }

    pub fn with_message_audit(mut self) -> Self {
        self.0 |= Self::MESSAGE_AUDIT;
        self
    }

    pub fn with_guild_messages(mut self) -> Self {
        self.0 |= Self::GUILD_MESSAGES;
        self
    }

    pub fn with_guild_message_reactions(mut self) -> Self {
        self.0 |= Self::GUILD_MESSAGE_REACTIONS;
        self
    }

    pub fn with_audio_action(mut self) -> Self {
        self.0 |= Self::AUDIO_ACTION;
        self
    }

    pub fn to_u64(self) -> u64 {
        self.0
    }
}

impl Default for Intents {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Event types (the `t` field on `op:0` dispatch)
// ---------------------------------------------------------------------------

/// Incoming event name from the gateway.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventType {
    // Ready / Resume
    Ready,
    Resumed,
    // Guild events
    GuildCreate,
    GuildUpdate,
    GuildDelete,
    ChannelCreate,
    ChannelUpdate,
    ChannelDelete,
    GuildMemberAdd,
    GuildMemberUpdate,
    GuildMemberRemove,
    // Message events
    #[allow(clippy::enum_variant_names)]
    MessageCreate,
    #[allow(clippy::enum_variant_names)]
    MessageDelete,
    #[allow(clippy::enum_variant_names)]
    AtMessageCreate,
    DirectMessageCreate,
    #[allow(clippy::enum_variant_names)]
    C2cMessageCreate,
    #[allow(clippy::enum_variant_names)]
    GroupAtMessageCreate,
    // Reactions
    MessageReactionAdd,
    MessageReactionRemove,
    // Group/C2C / Lifecycle
    FriendAdd,
    FriendDel,
    C2cMsgReceive,
    C2cMsgReject,
    GroupAddRobot,
    GroupDelRobot,
    GroupMsgReceive,
    GroupMsgReject,
    // Other
    Interaction,
    MessageAuditPass,
    MessageAuditReject,
    AudioStart,
    AudioFinish,
    AudioOnMic,
    AudioOffMic,
}

impl EventType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "READY" => Some(Self::Ready),
            "RESUMED" => Some(Self::Resumed),
            "GUILD_CREATE" => Some(Self::GuildCreate),
            "GUILD_UPDATE" => Some(Self::GuildUpdate),
            "GUILD_DELETE" => Some(Self::GuildDelete),
            "CHANNEL_CREATE" => Some(Self::ChannelCreate),
            "CHANNEL_UPDATE" => Some(Self::ChannelUpdate),
            "CHANNEL_DELETE" => Some(Self::ChannelDelete),
            "GUILD_MEMBER_ADD" => Some(Self::GuildMemberAdd),
            "GUILD_MEMBER_UPDATE" => Some(Self::GuildMemberUpdate),
            "GUILD_MEMBER_REMOVE" => Some(Self::GuildMemberRemove),
            "MESSAGE_CREATE" => Some(Self::MessageCreate),
            "MESSAGE_DELETE" => Some(Self::MessageDelete),
            "AT_MESSAGE_CREATE" => Some(Self::AtMessageCreate),
            "DIRECT_MESSAGE_CREATE" => Some(Self::DirectMessageCreate),
            "C2C_MESSAGE_CREATE" => Some(Self::C2cMessageCreate),
            "GROUP_AT_MESSAGE_CREATE" => Some(Self::GroupAtMessageCreate),
            "MESSAGE_REACTION_ADD" => Some(Self::MessageReactionAdd),
            "MESSAGE_REACTION_REMOVE" => Some(Self::MessageReactionRemove),
            "FRIEND_ADD" => Some(Self::FriendAdd),
            "FRIEND_DEL" => Some(Self::FriendDel),
            "C2C_MSG_RECEIVE" => Some(Self::C2cMsgReceive),
            "C2C_MSG_REJECT" => Some(Self::C2cMsgReject),
            "GROUP_ADD_ROBOT" => Some(Self::GroupAddRobot),
            "GROUP_DEL_ROBOT" => Some(Self::GroupDelRobot),
            "GROUP_MSG_RECEIVE" => Some(Self::GroupMsgReceive),
            "GROUP_MSG_REJECT" => Some(Self::GroupMsgReject),
            "INTERACTION_CREATE" => Some(Self::Interaction),
            "MESSAGE_AUDIT_PASS" => Some(Self::MessageAuditPass),
            "MESSAGE_AUDIT_REJECT" => Some(Self::MessageAuditReject),
            "AUDIO_START" => Some(Self::AudioStart),
            "AUDIO_FINISH" => Some(Self::AudioFinish),
            "AUDIO_ON_MIC" => Some(Self::AudioOnMic),
            "AUDIO_OFF_MIC" => Some(Self::AudioOffMic),
            _ => None,
        }
    }

    /// Whether this is a message event that should be routed to the agent loop.
    pub fn is_message_event(&self) -> bool {
        matches!(
            self,
            Self::C2cMessageCreate
                | Self::GroupAtMessageCreate
                | Self::AtMessageCreate
                | Self::DirectMessageCreate
                | Self::MessageCreate
        )
    }
}

// ---------------------------------------------------------------------------
// REST API message send types
// ---------------------------------------------------------------------------

/// Message type for REST send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum MsgType {
    #[serde(rename = "0")]
    Text = 0,
    Markdown = 2,
    Ark = 3,
    Embed = 4,
    Media = 7,
}

/// Reply mode for REST send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ReplyMode {
    #[serde(rename = "0")]
    AtSender = 0,
    #[serde(rename = "1")]
    AtAndReply = 1,
}

/// Markdown payload for `msg_type: 2`.
#[derive(Debug, Clone, Serialize)]
pub struct MarkdownPayload {
    pub template_id: Option<u64>,
    pub params: Vec<MarkdownParam>,
    pub variable: MarkdownVariable,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkdownParam {
    pub key: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkdownVariable(pub Vec<String>);

/// REST message send request (shared across C2C, group, guild).
#[derive(Debug, Clone, Serialize)]
pub struct SendMessageRequest {
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_type: Option<MsgType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_seq: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<MarkdownPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<Reply>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reply {
    #[serde(rename = "message.id")]
    pub message_id: String,
}

/// C2C proactive wakeup flag.
#[derive(Debug, Clone, Serialize)]
pub struct C2cWakeupRequest {
    pub content: String,
    #[serde(rename = "type")]
    pub msg_type: u32,
    #[serde(rename = "target_id")]
    pub openid: String,
    #[serde(rename = "is_batch")]
    pub is_batch: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_id: Option<String>,
}

/// File upload request for media (C2C / Group).
#[derive(Debug, Clone, Serialize)]
pub struct FileUploadRequest {
    pub file_type: u32,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_data: Option<String>,
    #[serde(rename = "srv_send_msg")]
    pub srv_send_msg: bool,
}

/// File type constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Photo = 1,
    Video = 2,
    Voice = 3,
    Doc = 4,
}

/// File upload response.
#[derive(Debug, Clone, Deserialize)]
pub struct FileUploadResponse {
    pub file_uuid: String,
    pub file_info: String,
    pub ttl: u32,
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_bitflags_or() {
        let intents = Intents::new()
            .with_guilds()
            .with_group_and_c2c()
            .with_direct_message();
        assert_eq!(intents.to_u64(), Intents::GUILDS | Intents::GROUP_AND_C2C | Intents::DIRECT_MESSAGE);
    }

    #[test]
    fn test_event_type_parse() {
        assert_eq!(EventType::from_str("READY"), Some(EventType::Ready));
        assert_eq!(EventType::from_str("C2C_MESSAGE_CREATE"), Some(EventType::C2cMessageCreate));
        assert_eq!(EventType::from_str("GROUP_AT_MESSAGE_CREATE"), Some(EventType::GroupAtMessageCreate));
        assert_eq!(EventType::from_str("UNKNOWN_EVENT"), None);
    }

    #[test]
    fn test_event_type_message_detection() {
        assert!(EventType::C2cMessageCreate.is_message_event());
        assert!(EventType::GroupAtMessageCreate.is_message_event());
        assert!(EventType::AtMessageCreate.is_message_event());
        assert!(!EventType::GuildCreate.is_message_event());
        assert!(!EventType::FriendAdd.is_message_event());
    }

    #[test]
    fn test_op_code_serde_dispatch() {
        let json = r#"{"op":0,"s":42,"t":"TEST","d":{}}"#;
        let msg: WsIncomingMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.op, OpCode::Dispatch);
        assert_eq!(msg.s, Some(42));
        assert_eq!(msg.t, Some("TEST".to_string()));
    }

    #[test]
    fn test_identify_payload_serialization() {
        let payload = IdentifyPayload {
            token: "QQBot abc123".to_string(),
            intents: Intents::GUILDS | Intents::GROUP_AND_C2C,
            shard: None,
            properties: Properties::default(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"token\":\"QQBot abc123\""));
        assert!(json.contains("\"intents\":33554433"));
        assert!(!json.contains("\"shard\""));
    }

    #[test]
    fn test_close_code_classification() {
        assert!(CloseCode::from_code(4006).requires_reidentify());
        assert!(CloseCode::from_code(4004).requires_reidentify());
        assert!(CloseCode::from_code(4009).should_resume());
        assert!(CloseCode::from_code(4008).should_resume());
        assert!(CloseCode::from_code(4914).is_fatal());
        assert!(CloseCode::from_code(4915).is_fatal());
        assert!(!CloseCode::from_code(4006).is_fatal());
    }
}
