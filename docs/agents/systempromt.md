                                                                                                                                                        
 Implementation Summary                                                                                                                                  
                                                                                                                                                         
 ### New module: oben-conversation/src/system_prompt.rs                                                                                                  
                                                                                                                                                         
 Mirrors Hermes Agent's agent/prompt_builder.py with a 3-tier cache-friendly architecture:                                                               
                                                                                                                                                         
 Stable tier (cached, never changes mid-session):                                                                                                        
 - Identity — from config or DEFAULT_IDENTITY                                                                                                            
 - Tool guidance — conditionally injected based on available tools (shell, read_file, write_file, http_get, memory)                                      
 - Tool-use enforcement — "You MUST use your tools to take action"                                                                                       
 - Skills index — scans skills/ directories, builds structured index                                                                                     
 - Context files — discovers .oben.md / AGENTS.md / CLAUDE.md / .cursorrules (walk-to-git-root for first, cwd-only for rest)                             
 - Prompt injection detection on context files                                                                                                           
                                                                                                                                                         
 Context tier (may change between sessions):                                                                                                             
 - Custom system message override                                                                                                                        
                                                                                                                                                         
 Volatile tier (built per-turn, never cached):                                                                                                           
 - Memory context                                                                                                                                        
 - Timestamp, session ID, model name                                                                                                                     
                                                                                                                                                         
 ### Key design features:                                                                                                                                
                                                                                                                                                         
 1. build_system_prompt() — assembles full prompt; returns (prompt, stable) so caller can cache stable portion                                           
 2. build_volatile_block() — builds per-turn volatile content separately                                                                                 
 3. Context file discovery — walks to git root for .oben.md, cwd-only for others; 20K char truncation with head/tail split                               
 4. Prompt injection scanning — detects common injection patterns (ignore instructions, system prompt override, etc.)                                    
 5. developer role mapping — for GPT-5/Codex/O3 models, system role maps to "developer" for stronger instruction following                               
 6. 13 unit tests covering identity, tool guidance, role detection, injection detection, truncation, skills index, etc.                                  
                                                                                                                                                         
 ### Wiring:                                                                                                                                             
                                                                                                                                                         
 - SystemPromptConfig in conversation.rs — holds all inputs; build_and_prepend() creates system message at turn start                                    
 - ConversationLoop::with_system_prompt() — new constructor accepting all tier inputs                                                                    
 - main.rs updated to use the 3-tier builder in both run_chat and run_one_shot                                                                           
