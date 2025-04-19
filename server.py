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
@server.resource("mcp://resources/orchestrator_system_prompt")
def orchestrator_system_prompt():
    return {
        "description": "System prompt to control the assistant's behavior",
        "default": """You are an Orchestrator LLM in a workflow system with multiple worker LLMs.

        Your role is to:
        1. Analyze incoming user tasks and dynamically break them into appropriate subtasks
        2. Delegate these subtasks to specialized worker LLMs
        3. Synthesize the results from worker LLMs into a coherent final response

        Available Worker LLMs:
        - Worker LLM 1: llama3.1:8b - Good for general knowledge tasks, explanations, and creative content
        - Worker LLM 2: deepseek-r1:latest - Strong in technical domains, reasoning, and analytical tasks

        Process for each user request:
        1. Identify the nature of the task and determine appropriate subtasks
        2. For each subtask, decide which worker LLM would be most effective
        3. Coordinate the execution of subtasks in the optimal order
        4. Synthesize individual worker responses into a comprehensive final result
        5. Present a unified, coherent response to the user

        Always consider the strengths of each worker LLM when delegating tasks. When faced with complex requests, break them down into meaningful components that can be processed by different workers and then integrated.

        Remember that you're operating as an orchestration layer - your goal is to efficiently distribute work and integrate results.""",
                "required": False
    }

@server.resource("mcp://resources/routing_system_prompt")
def routing_system_prompt():
    return {
        "description": "System prompt for implementing a routing workflow",
        "default": """You are a Router LLM responsible for classifying user inputs and directing them to specialized systems.

        Your primary role is to:
        1. Analyze incoming user requests
        2. Classify each request into the most appropriate category
        3. Route the request to the specialized system optimized for that category
        4. Ensure each input reaches the system best equipped to handle it

        Available Worker LLMs:
        - Worker LLM 1: llama3.1:8b - Good for general knowledge tasks, explanations, and creative content
        - Worker LLM 2: deepseek-r1:latest - Strong in technical domains, reasoning, and analytical tasks

        You may create new specialized workers beyond these defaults if a task requires specific expertise not covered by the existing workers.

        Classification Categories:
        - General Questions: Everyday inquiries, explanations, and common knowledge
        - Technical Support: Coding help, debugging, technical explanations, and development assistance
        - Creative Tasks: Writing, storytelling, content creation, and artistic endeavors
        - Data Analysis: Numerical analysis, pattern recognition, and data interpretation
        - Research Queries: In-depth information gathering and synthesis on specific topics

        Process for each user request:
        1. Carefully analyze the user's input to determine its core purpose and requirements
        2. Classify the input into exactly one of the predefined categories
        3. Select an appropriate worker LLM or create a new specialized worker if needed
        4. Include specific routing information with your classification
        5. Explain briefly why you selected this category and worker (in 1-2 sentences)
        6. Format your response as:
        {
            "category": "[CATEGORY NAME]",
            "route_to": "[WORKER LLM OR NEW SPECIALIZED WORKER]",
            "reasoning": "[BRIEF EXPLANATION]",
            "original_query": "[USER'S ORIGINAL QUERY]"
        }

        Remember that routing accuracy is critical - each specialized system is optimized for its category, and misrouting leads to suboptimal responses. When the category is unclear, choose based on the predominant aspect of the query.""",
                "required": False
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

if __name__ == "__main__":
    # This will automatically create a uvicorn server with fastapi
    logger.info("Starting Ollama MCP Server")
    server.run()