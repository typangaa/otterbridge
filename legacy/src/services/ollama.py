import os
import json
import aiohttp
import logging
from typing import Dict, List, Any, AsyncGenerator, Optional

logger = logging.getLogger("ollama_service")

class OllamaService:
    """Service for interacting with Ollama API"""
    
    def __init__(self, base_url: str = None):
        """Initialize the Ollama service with base URL"""
        self.base_url = base_url or os.getenv("OLLAMA_API_URL", "http://localhost:11434")
        logger.info(f"Initialized OllamaService with base URL: {self.base_url}")
    
    async def list_models(self) -> List[Dict[str, Any]]:
        """List all available models from Ollama server"""
        logger.debug("Listing models from Ollama API")
        async with aiohttp.ClientSession() as session:
            async with session.get(f"{self.base_url}/api/tags") as response:
                if response.status != 200:
                    error_text = await response.text()
                    logger.error(f"Failed to list models: {error_text}")
                    raise Exception(f"Failed to list models: {error_text}")
                
                data = await response.json()
                logger.debug(f"Retrieved {len(data.get('models', []))} models")
                return data.get("models", [])
    
    async def get_model_info(self, model_name: str) -> Dict[str, Any]:
        """Get detailed information about a specific model"""
        logger.debug(f"Getting info for model: {model_name}")
        async with aiohttp.ClientSession() as session:
            # First try the pull endpoint to get model info
            async with session.post(
                f"{self.base_url}/api/show",
                json={"name": model_name}
            ) as response:
                if response.status == 200:
                    data = await response.json()
                    return data
                else:
                    error_text = await response.text()
                    logger.warning(f"Failed to get model info via show API: {error_text}")
                    
                    # Fallback to checking if model exists in list
                    models = await self.list_models()
                    for model in models:
                        if model.get("name") == model_name:
                            return model
                    
                    raise Exception(f"Model {model_name} not found")
    
    async def generate_completion(
        self,
        model: str,
        prompt: str,
        stream: bool = False,
        **kwargs
    ) -> AsyncGenerator[Dict[str, Any], None]:
        """Send a generate completion request to Ollama API"""
        logger.debug(f"Generate completion request for model {model}")
        
        # Prepare payload
        payload = {
            "model": model,
            "prompt": prompt,
            "stream": stream,
            **kwargs
        }
        
        async with aiohttp.ClientSession() as session:
            async with session.post(f"{self.base_url}/api/generate", json=payload) as response:
                if response.status != 200:
                    error_text = await response.text()
                    logger.error(f"Generate completion error: {error_text}")
                    raise Exception(f"Generate completion error: {error_text}")
                
                if stream:
                    # Process the streaming response
                    async for line in response.content:
                        line = line.strip()
                        if not line:
                            continue
                        
                        try:
                            data = json.loads(line)
                            logger.debug("Received stream chunk")
                            yield data
                        except json.JSONDecodeError as e:
                            logger.error(f"Error decoding JSON: {e}")
                            continue
                else:
                    # Process the complete response
                    data = await response.json()
                    logger.debug("Received complete response")
                    yield data

    async def chat_completion(
        self,
        model: str,
        messages: List[Dict[str, str]],
        system: Optional[str] = None,
        stream: bool = False,
        **kwargs
    ) -> AsyncGenerator[Dict[str, Any], None]:
        """Send a chat completion request to Ollama API"""
        logger.debug(f"Chat completion request for model {model}")
        
        # Prepare payload
        payload = {
            "model": model,
            "messages": messages,
            "stream": stream,
            **kwargs
        }
        
        if system:
            payload["system"] = system
        
        async with aiohttp.ClientSession() as session:
            async with session.post(f"{self.base_url}/api/chat", json=payload) as response:
                if response.status != 200:
                    error_text = await response.text()
                    logger.error(f"Chat completion error: {error_text}")
                    raise Exception(f"Chat completion error: {error_text}")
                
                if stream:
                    # Process the streaming response
                    async for line in response.content:
                        line = line.strip()
                        if not line:
                            continue
                        
                        try:
                            data = json.loads(line)
                            logger.debug("Received stream chunk")
                            yield data
                        except json.JSONDecodeError as e:
                            logger.error(f"Error decoding JSON: {e}")
                            continue
                else:
                    # Process the complete response
                    data = await response.json()
                    logger.debug("Received complete response")
                    yield data
    
