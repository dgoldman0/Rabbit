# Rabbit Demo Warren

This demo warren showcases Rabbit Protocol v1.1 features across three interconnected burrows.

## What's Included

**Burrow 0 (port 7443)** — Warren Hub
- Comprehensive documentation pages
- Navigation through all content types
- AI chat integration example (requires API key)
- GUI theme configuration

**Burrow 1 (port 7444)** — Content Examples
- Text and markdown rendering
- Code samples with syntax
- Event stream demonstrations
- Light theme configuration

**Burrow 2 (port 7445)** — Community Space
- Pub/sub event topics
- Real-time chat channels
- Community guidelines
- System theme (follows OS)

## Running the Demo

### 1. Build the Warren Launcher

```bash
cd rabbit_engine
cargo build --release
```

For GUI support (optional):
```bash
cargo build --release --features gui
```

### 2. Start the Warren

From the Rabbit project root:

```bash
cd rabbit_engine
./target/release/rabbit-warren --config-dir ../demo-warren
```

The warren will start all three burrows:
- Burrow 0: http://127.0.0.1:7443
- Burrow 1: http://127.0.0.1:7444
- Burrow 2: http://127.0.0.1:7445

### 3. Browse with Terminal Client

```bash
# Browse the warren hub
./target/release/rabbit browse 127.0.0.1:7443

# Or browse content examples
./target/release/rabbit browse 127.0.0.1:7444

# Or browse community space
./target/release/rabbit browse 127.0.0.1:7445
```

Navigate using:
- Number keys to select menu items
- `b` to go back
- `q` to quit

### 4. Browse with GUI (if built with --features gui)

```bash
./target/release/rabbit-gui 127.0.0.1:7443
```

The GUI will:
- Generate AI-driven HTML views from Gopher menus
- Apply theme-aware CSS styling
- Enable click navigation through content
- Show connection status and navigation breadcrumbs

### 5. Subscribe to Event Topics

```bash
# Subscribe to general chat
./target/release/rabbit sub 127.0.0.1:7445 /q/general

# Subscribe to announcements
./target/release/rabbit sub 127.0.0.1:7445 /q/announcements

# Subscribe from sequence 1 (get history)
./target/release/rabbit sub 127.0.0.1:7445 /q/general --since 1
```

Events published to topics will appear in real-time.

## Enabling AI Chat (Optional)

To enable AI-powered chat in Burrow 0:

1. Set your OpenAI API key:
   ```bash
   export OPENAI_API_KEY="sk-..."
   ```

2. Uncomment the `[[ai.chats]]` section in `demo-warren/burrow-0/config.toml`

3. Restart the warren

4. Subscribe to the chat topic:
   ```bash
   ./target/release/rabbit sub 127.0.0.1:7443 /q/chat
   ```

5. Publish a message:
   ```bash
   echo "Hello AI!" | ./target/release/rabbit pub 127.0.0.1:7443 /q/chat
   ```

The AI will respond based on the configured system message and conversation history.

## Warren Discovery

Visit `/warren` on any burrow to see the directory of all burrows in the warren:

```bash
./target/release/rabbit browse 127.0.0.1:7443
# Select "Warren Directory" from menu
```

This demonstrates the warren discovery protocol where each burrow can find its peers.

## Features Demonstrated

✅ **Core Protocol**
- Gopher-style menus and navigation
- Text content delivery
- Markdown rendering (type 't')
- Event pub/sub (type 'q')

✅ **Warren System**
- Multi-burrow networks
- Peer discovery via /warren
- Per-burrow configuration

✅ **AI Integration**
- Chat topic AI responses
- Conversation memory and context
- System message customization

✅ **GUI Rendering**
- AI-driven HTML generation from menus
- Theme-aware CSS styling
- Click-based navigation
- Connection status indicators

✅ **Event System**
- Real-time event delivery
- Persistent continuity engine
- Replay from sequence numbers
- Multiple subscribers per topic

## Troubleshooting

**Port already in use:**
Edit the `port` values in the config.toml files to use different ports.

**GUI doesn't start:**
Make sure you built with `--features gui` and have system dependencies:
```bash
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev libxdo-dev
```

**AI chat not working:**
- Check that `OPENAI_API_KEY` is set
- Ensure the `[[ai.chats]]` section is uncommented
- Verify network connectivity to OpenAI

**Events not delivering:**
- Make sure the topic exists in the config (under `[[content.topics]]`)
- Check that the burrow is running on the correct port
- Verify the topic path starts with `/q/`

## Next Steps

- Explore the documentation pages in Burrow 0
- Try different content types in Burrow 1
- Publish and subscribe to events in Burrow 2
- Enable AI chat and have a conversation
- Compare terminal vs GUI browsing experience
- Modify configs to add your own content

Enjoy exploring the Rabbit Protocol!
