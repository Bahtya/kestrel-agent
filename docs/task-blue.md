# 🔵 Blue Hat — Process & Architecture Overview

You are the BLUE HAT thinker analyzing the Hermes Agent self-evolution system.

## Your Role
Blue Hat manages the thinking process. You focus on the OVERALL ARCHITECTURE — how all the self-evolution pieces fit together into a coherent system. You map the data flow, identify the control loops, and understand the system as a whole.

## What to Analyze

Read the Hermes Agent source code at `/opt/hermes-research/` and answer these questions:

### 1. Self-Evolution Closed Loop Architecture
Map the complete feedback loop: User interaction → behavior recording → evaluation → strategy optimization → skill crystallization → capability enhancement. Where does each step happen in the code? What files/functions implement each stage?

### 2. KEPA/GEPA Engine
Find the "backpropagation-like" mechanism. How does the system periodically review execution records? What triggers a review (every ~15 tasks? timer-based? event-driven?)? What does it generate as output?

### 3. Auto-Skill Generation Pipeline
How are multi-step task solutions abstracted into reusable skill documents? What's the skill format (Markdown)? What metadata do skills contain? Where are they stored? How are they matched and loaded for future tasks?

### 4. Memory System Architecture
How does memory persist across sessions? What's stored (user preferences, task outcomes, correction patterns)? How is memory injected into context? What's the relationship between memory and skills?

### 5. Context Engineering
How does Hermes build its system prompt dynamically? What sections exist (identity, runtime metadata, memory, skills, custom instructions)? How does JIT loading work?

### 6. Integration Points
Map the dependency chain: which modules depend on which? How do the self-evolution components integrate with the core agent loop?

## Key Files to Read
- `run_agent.py` — core agent loop, conversation flow
- `agent/prompt_builder.py` — system prompt assembly
- `agent/context_compressor.py` — context management
- `tools/` — tool registry, all tool implementations
- `cron/` — scheduled tasks (self-review triggers?)
- `agent/skill_commands.py` — skill loading and injection
- `cli.py` — session management, memory integration
- `gateway/run.py` — gateway loop, event handling

## Output
Write a comprehensive architecture document to `/tmp/hats/01-blue-hat-architecture.md` with:
1. System overview diagram (ASCII)
2. Data flow map for the self-evolution loop
3. Module dependency graph
4. Complete file-by-file analysis of self-evolution related code
5. Key data structures and their relationships
6. Summary: what makes this system "self-evolving" vs a traditional agent

Write in Chinese. Be thorough — this document will guide the porting effort.
