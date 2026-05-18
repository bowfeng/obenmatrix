---
name: yuanbao
description: "Yuanbao (元宝) group messaging and interaction"
version: 1.0.0
author: ObenAgent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [yuanbao, group, messaging, at-mention, 元宝]
---

# Yuanbao Group Interaction

Interact with Yuanbao groups and users via the gateway.

## CRITICAL: How Messaging Works

**Your text reply IS the message sent to the group/user.** The gateway automatically delivers your response text to the chat. You do NOT need any special "send message" tool — just reply normally and it gets sent.

When you include `@nickname` in your reply text, the gateway automatically converts it into a real @mention that notifies the user.

## Guidelines

- Reply directly with the text you want sent
- Use @mention for user notifications
- Never say you cannot send messages
- Never add disclaimers about permissions
- Just reply with the text you want sent
