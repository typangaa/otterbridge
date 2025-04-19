## File Structure

```
├── .env.example           # Example environment variables
├── .gitignore             # Files to exclude from git
├── LICENSE                # Open source license (MIT, Apache, etc.)
├── README.md              # Project documentation
├── server.py              # MCP server implementation (previously fastmcp_server.py)
├── requirements.txt       # Python dependencies
└── src/                   # Source code directory
    ├── __init__.py        # Package initialization
    └── services/          # Services
        ├── __init__.py
        └── ollama.py      # Ollama service
```# OtterBridge

<img src="https://via.placeholder.com/200x200.png?text=OtterBridge" alt="OtterBridge Logo" width="200" align="right"/>

OtterBridge is a lightweight, flexible server for connecting applications to various Large Language Model providers. Following the principles of simplicity and composability outlined in Anthropic's guide to building effective agents, OtterBridge provides a clean interface to LLMs while maintaining adaptability for different use cases.

Currently supporting Ollama, with planned expansions to support other providers like ChatGPT and Claude.

## Features

- **Provider-Agnostic**: Designed to work with multiple LLM providers (currently Ollama, with ChatGPT and Claude coming soon)
- **Simple, Composable Design**: Following best practices for LLM agent architecture
- **Lightweight Server**: Built with FastMCP for reliable, efficient server implementation
- **API-First Design**: Clean JSON API for easy integration with any client application
- **Streaming Support**: Works with both streaming and non-streaming responses
- **Model Management**: Easy access to model information and capabilities

## Why "OtterBridge"?

Like otters who build connections between riverbanks, OtterBridge creates seamless pathways between your applications and various LLM providers. Just as otters are adaptable and resourceful, OtterBridge adapts to different LLM backends while providing consistent interfaces.

## Prerequisites

Before installing OtterBridge, you need to have:

1. [Ollama](https://ollama.ai/download) installed and running on the default port
2. [uv](https://github.com/astral-sh/uv) installed for Python package management

## Installation

1. Clone this repository:
```bash
git clone https://github.com/yourusername/otterbridge.git
cd otterbridge
```

2. Install dependencies using uv:
```bash
uv add -r requirements.txt
```

3. Create a `.env` file based on the provided `.env.example`:
```bash
cp .env.example .env
```

4. Configure your environment variables in the `.env` file.

## Claude Desktop Integration

For Claude Desktop users, you'll need to add OtterBridge to your Claude Desktop configuration:

1. Open your Claude Desktop config file
2. Add the following configuration (adjust the path to match your local installation):

```json
"otterbridge": {
    "command": "uv",
    "args": [
        "--directory",
        "C:\\Path\\To\\Your\\otterbridge",
        "run",
        "server.py"
    ]
}
```

## Usage

### Starting the Server

OtterBridge can be started in two ways:

1. **Manual start:**
```bash
uv run server.py
```

2. **Automatic start with MCP clients:** 
   - When using compatible MCP clients like Claude Desktop, OtterBridge will start automatically when needed

The server will be available at http://localhost:8000 (default port) and connect to Ollama running on its default port.

### Available Tools

OtterBridge exposes the following tools via the Model Context Protocol (MCP):

- **chat**: Send messages to LLMs and get AI-generated responses
- **list_models**: Retrieve information about available language models

## Tool Usage Examples

### List Available Models

```python
import requests

response = requests.post("http://localhost:8000/tools/list_models", json={
    "context": {}
})
print(response.json())
```

Example response:
```json
{
    "status": "connected",
    "server_status": "online",
    "available_models": ["llama3", "llama3.1:8b", "codellama", "llama3.3", "qwen2.5"],
    "available_models_count": 5,
    "message": "Successfully retrieved available Ollama models"
}
```

### Chat Completion

```python
import requests

response = requests.post("http://localhost:8000/tools/chat", json={
    "context": {
        "session_id": "user123",
        "resources": {
            "model": "llama3:latest", 
            "system_prompt": "You are a helpful assistant."
        }
    },
    "messages": [
        {"role": "user", "content": "Hello, how are you today?"}
    ]
})
print(response.json())
```

Example response:
```json
{
    "role": "assistant",
    "content": "I'm doing well, thank you for asking! I'm here and ready to help you with any questions or tasks you might have. How can I assist you today?",
    "model": "llama3:latest"
}
```

## Configuration

OtterBridge can be configured using environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `OLLAMA_BASE_URL` | URL of the Ollama server | http://localhost:11434 |
| `DEFAULT_MODEL` | Default model to use | llama3.3 |

## Roadmap

- **Q2 2025**: Support for ChatGPT API integration
- **Q3 2025**: Support for Claude API integration

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

### Development Guidelines

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add some amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## License

This project is licensed under the [MIT License](LICENSE).

## Acknowledgements

- [MCP (Model Context Protocol)](https://github.com/modelcontextprotocol) for the server framework
- [Ollama](https://github.com/ollama/ollama) for local LLM hosting
- [Anthropic's guide to building effective agents](https://www.anthropic.com/engineering/building-effective-agents) for architectural inspiration
# otterbridge
OtterBridge is a lightweight, mcp server for connecting applications to various Large Language Model providers.
