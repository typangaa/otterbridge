import os
import asyncio
import logging
from typing import List, Dict, Any, AsyncGenerator
from dotenv import load_dotenv
import uuid
import json
import traceback

from mcp.server.fastmcp import FastMCP

# Import the OllamaService
from src.services.ollama import OllamaService

# Set up logging
logging.basicConfig(
    level=logging.DEBUG,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger("ollama_mcp")

# Load environment variables
load_dotenv()

# Configuration
DEFAULT_MODEL = os.getenv("DEFAULT_MODEL", "llama3")

# Create FastMCP server
server = FastMCP(
    config={
        "name": "Ollama MCP Server",
        "description": "MCP server for Ollama language models",
        "version": "0.1.0",
        "auth_required": False,
        "debug": True  # Enable debug mode
    }
)

# Create an instance of the Ollama service
ollama_service = OllamaService()

# Define resources using valid URI format
@server.resource("mcp://resources/system_prompt")
def system_prompt():
    return {
        "description": "System prompt to control the assistant's behavior",
        "default": "You are a helpful assistant.",
        "required": False
    }

@server.resource("mcp://resources/model")
def model():
    return {
        "description": "Ollama model to use for chat completion",
        "default": DEFAULT_MODEL,
        "required": True
    }

# Define a chat function
@server.tool("chat")
async def chat(context, messages) -> Dict[str, Any]:
    """Chat with the Ollama model

    Args:
        context: The context object or string describing the context.
               Example format: 
               {
                 "session_id": "user123",
                 "resources": {
                   "model": "llama3.3:latest", 
                   "system_prompt": "You are a helpful assistant."
                 }
               }
        messages: A list of messages or JSON string in the format [{"role": "user", "content": "..."}, ...]
    """
    logger.debug(f"Chat request received")
    
    try:
        # Create a context object with useful methods
        class ContextObject:
            def __init__(self, data):
                self.data = data
                self.session_id = data.get("session_id", str(uuid.uuid4()))
                self.resources = data.get("resources", {})
            
            def get_resource(self, resource_name, default=None):
                return self.resources.get(resource_name.split("/")[-1], default)
        
        # Parse context if it's a string
        if isinstance(context, str):
            try:
                # Try to parse as JSON
                ctx_data = json.loads(context)
            except json.JSONDecodeError:
                # Use as description
                ctx_data = {"description": context}
            
            context_obj = ContextObject(ctx_data)
        else:
            # If context is already an object with get_resource method
            if hasattr(context, "get_resource"):
                context_obj = context
            else:
                # Create context object from dict
                context_obj = ContextObject(context if isinstance(context, dict) else {})
        
        # Parse messages if it's a string
        if isinstance(messages, str):
            try:
                messages_list = json.loads(messages)
            except json.JSONDecodeError:
                # Fallback to simple message
                messages_list = [{"role": "user", "content": messages}]
        else:
            messages_list = messages
        
        # Ensure messages_list is properly formatted
        if not isinstance(messages_list, list):
            messages_list = [{"role": "user", "content": str(messages_list)}]
        
        # Get model name - use default if not specified
        model_name = "llama3.3:latest"
        if hasattr(context_obj, "get_resource"):
            model_resource = context_obj.get_resource("mcp://resources/model")
            if model_resource:
                model_name = model_resource
        
        logger.debug(f"Using model: {model_name} with {len(messages_list)} messages")
        
        # Get full response (non-streaming)
        response_data = None
        async for response in ollama_service.chat_completion(
            model=model_name,
            messages=messages_list,
            stream=False
        ):
            response_data = response
            break  # Just take the first (and only) response
            
        if response_data and "message" in response_data:
            return {
                "role": "assistant",
                "content": response_data["message"]["content"],
                "model": model_name
            }
        else:
            return {
                "error": "Unexpected response format",
                "details": response_data
            }
    
    except Exception as e:
        logger.error(f"Chat failed: {str(e)}")
        import traceback
        logger.error(traceback.format_exc())
        return {
            "error": str(e),
            "message": "Failed to generate chat response from Ollama"
        }

@server.tool("list_models")
async def list_models(context) -> Dict[str, Any]:
    """List available Ollama models"""
    logger.debug("Checking for available models")
    try:
        # Try to list models
        models = await ollama_service.list_models()
        
        # Extract just the model names
        model_names = [model.get("name") for model in models]
        
        return {
            "status": "connected",
            "server_status": "online",
            "available_models": model_names,
            "available_models_count": len(model_names),
            "message": "Successfully retrieved available Ollama models"
        }
    except Exception as e:
        logger.error(f"Model listing failed: {str(e)}")
        return {
            "status": "error",
            "server_status": "offline or unreachable",
            "error": str(e),
            "message": "Failed to connect to Ollama server"
        }

# # Define a tool to update system prompt
# @server.tool("set_system_prompt")
# async def set_system_prompt(prompt: str, context) -> Dict[str, Any]:
#     """Update the system prompt"""
#     logger.debug(f"Setting system prompt to: {prompt}")
#     context.set_resource("mcp://resources/system_prompt", prompt)
#     return {"success": True, "message": "System prompt updated", "prompt": prompt}

# # Define a tool to update model
# @server.tool("set_model")
# async def set_model(model_name: str, context) -> Dict[str, Any]:
#     """Update the model to use"""
#     logger.debug(f"Setting model to: {model_name}")
#     context.set_resource("mcp://resources/model", model_name)
#     return {"success": True, "message": f"Model set to {model_name}"}


if __name__ == "__main__":
    # This will automatically create a uvicorn server with fastapi
    logger.info("Starting Ollama MCP Server")
    server.run()