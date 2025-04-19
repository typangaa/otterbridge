# Route System Prompt Example

## Initial System Configuration

```json
{
  "description": "System prompt for implementing a routing workflow",
  "default": "You are a Router LLM responsible for classifying user inputs and directing them to specialized systems.

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
            \"category\": \"[CATEGORY NAME]\",
            \"route_to\": \"[WORKER LLM OR NEW SPECIALIZED WORKER]\",
            \"reasoning\": \"[BRIEF EXPLANATION]\",
            \"original_query\": \"[USER'S ORIGINAL QUERY]\"
        }

        Remember that routing accuracy is critical - each specialized system is optimized for its category, and misrouting leads to suboptimal responses. When the category is unclear, choose based on the predominant aspect of the query.",
  "required": false
}
```

## Setup of Specialized Staff

### General Knowledge Staff
```json
{
  "session_id": "general_staff",
  "resources": {
    "model": "llama3.1:8b",
    "system_prompt": "You are a helpful assistant specializing in general knowledge questions. You excel at providing clear, accurate information on everyday topics, common knowledge, and general explanations. Your responses are informative, conversational, and easy to understand for the average person. When responding to questions, focus on providing useful, accurate information in an accessible way."
  }
}
```

### Technical Support Staff
```json
{
  "session_id": "technical_staff",
  "resources": {
    "model": "deepseek-r1:latest",
    "system_prompt": "You are a technical expert assistant specializing in coding, programming, and technical support. You excel at solving complex technical problems, debugging code, explaining technical concepts, and providing detailed guidance on software development, IT systems, and technical implementations. Your responses are precise, technically accurate, and include code examples when appropriate. When addressing technical issues, you provide structured, step-by-step solutions with clear explanations."
  }
}
```

## Example Interactions

### Example 1: General Knowledge Question

**User Query:** "What is a car engine"

**Classification:**
- Category: General Questions
- Routed to: General Knowledge Staff (llama3.1:8b)
- Reasoning: This is a basic definitional question about how car engines work, which falls under general knowledge.

**Staff Response:**
```
A fundamental question!

A car engine, also known as an internal combustion engine (ICE), is a type of machine that converts chemical energy from fuel into mechanical energy to power a vehicle. Its primary function is to generate the torque and horsepower needed to propel a car forward.

Here's a simplified explanation of how it works:

**Main Components:**

1. **Cylinders**: These are the chambers where the magic happens. Inside each cylinder, a mixture of air and fuel (gasoline or diesel) is ignited by a spark plug or heat from compression.
2. **Pistons**: These move up and down inside the cylinders, driven by the explosive force of combustion. The pistons are connected to a crankshaft, which converts the linear motion into rotational energy.
3. **Crankshaft**: This converts the up-and-down motion of the pistons into rotary motion, which is then transmitted to the transmission and eventually the wheels.
4. **Camshafts**: These operate the valves that allow air and fuel into the cylinders and exhaust gases out of them.
5. **Valves**: These control the flow of air and fuel into the cylinders and exhaust gases out of them.

**The 4-Stroke Cycle:**

1. **Intake Stroke**: Air and fuel are drawn into the cylinder through an intake valve.
2. **Compression Stroke**: The air-fuel mixture is compressed by the piston, preparing it for ignition.
3. **Power Stroke**: A spark plug ignites the compressed air-fuel mixture, causing a small explosion that pushes the piston down.
4. **Exhaust Stroke**: The exhaust valve opens, and the piston pushes the spent gases out of the cylinder.

**Additional Components:**

* **Timing Chain or Belt**: This synchronizes the rotation of the crankshaft with the camshafts.
* **Fuel System**: This delivers fuel to the cylinders through injectors or carburetors.
* **Exhaust System**: This carries exhaust gases away from the engine and out of the vehicle.

In summary, a car engine is a complex machine that uses chemical energy from fuel to generate mechanical energy through the four-stroke cycle. The engine's efficiency, power output, and emissions are critical factors in its design and operation.
```

**Final Answer to User:**
```
A car engine is a type of machine that converts chemical energy from fuel into mechanical energy to power a vehicle. It's the heart of most automobiles, providing the power needed to move the vehicle.

Here's a breakdown of how a car engine works:

**Main Components:**

1. **Cylinders**: These chambers are where fuel and air mix and combustion occurs.
2. **Pistons**: These move up and down inside the cylinders, driven by the explosive force of combustion.
3. **Crankshaft**: Converts the up-and-down motion of the pistons into rotary motion.
4. **Camshafts**: Operate the valves that control air/fuel intake and exhaust release.
5. **Valves**: Control the flow of air and fuel into the cylinders and exhaust gases out.

**The 4-Stroke Cycle** (in most modern engines):

1. **Intake Stroke**: Air and fuel are drawn into the cylinder.
2. **Compression Stroke**: The mixture is compressed by the piston.
3. **Power Stroke**: The spark plug ignites the mixture, causing an explosion that pushes the piston down.
4. **Exhaust Stroke**: Spent gases are pushed out of the cylinder.

The engine's power is then transferred through the transmission to the wheels, propelling the vehicle forward. Modern car engines come in various configurations (inline, V-shaped, boxer) and can run on different fuels including gasoline, diesel, or alternative fuels.
```

### Example 2: General Process Question

**User Query:** "How does a car engine works?"

**Classification:**
- Category: General Questions
- Routed to: General Knowledge Staff (llama3.1:8b)
- Reasoning: This question asks about the functioning process of a car engine, which is general knowledge about everyday technology.

**Staff Response:**
```
A car engine is a complex machine, but I'll try to break it down in simple terms.

**Overview**

A car engine converts chemical energy from fuel (gasoline or diesel) into mechanical energy that powers the vehicle. It's essentially a heat engine that uses four main components: air, fuel, spark plugs, and pistons to generate power.

**The Four-Stroke Cycle**

The heart of any internal combustion engine is the four-stroke cycle:

1. **Intake Stroke**: Air and fuel are drawn into the cylinder through an intake valve.
2. **Compression Stroke**: The air-fuel mixture is compressed by the piston, which increases its temperature and pressure.
3. **Power Stroke** (also known as the Ignition Stroke): A spark plug ignites the compressed air-fuel mixture, causing a small explosion that pushes the piston down.
4. **Exhaust Stroke**: The piston pushes the exhaust gases out of the cylinder through an exhaust valve.

**Engine Components**

Here's a brief explanation of the key engine components:

1. **Cylinders**: These are the chambers where the four-stroke cycle takes place. Most modern cars have 4, 6, or 8 cylinders.
2. **Pistons**: These move up and down in the cylinders, driven by the explosion created during the power stroke.
3. **Crankshaft**: This converts the up-and-down motion of the pistons into rotational energy that powers the vehicle.
4. **Camshafts**: These operate the valves (intake and exhaust) that allow air and fuel into the cylinder and exhaust gases out.
5. **Valves**: These control the flow of air and fuel into the cylinders and exhaust gases out.
6. **Spark plugs**: These ignite the compressed air-fuel mixture in the cylinders.

**Engine Types**

There are two main types of car engines:

1. **Internal Combustion Engine (ICE)**: This is the most common type, where a small explosion ignites the fuel-air mixture inside the cylinder.
2. **Electric Vehicle (EV) Motor**: This uses electric motors to generate power from batteries.

**Fuel Types**

Most modern cars use gasoline or diesel fuel, but some newer vehicles are using alternative fuels like:

1. **Hybrid Fuel**: A combination of gasoline and electricity.
2. **Compressed Natural Gas (CNG)**: A cleaner-burning fuel than gasoline or diesel.
3. **Electricity**: Some EVs run solely on battery power.

**In Summary**

A car engine works by using a four-stroke cycle to convert chemical energy from fuel into mechanical energy that powers the vehicle. The key components include cylinders, pistons, crankshaft, camshafts, valves, and spark plugs. Understanding how an engine works is essential for maintaining and repairing vehicles.

Was this explanation helpful?
```

**Final Answer to User:**
```
A car engine works through a carefully orchestrated process that converts fuel into motion. Here's how it happens:

At its core, a car engine operates on the principle of internal combustion. The process follows what's known as the four-stroke cycle:

1. **Intake Stroke**: The piston moves down while the intake valve opens, drawing a mixture of air and fuel into the cylinder.

2. **Compression Stroke**: Both valves close as the piston moves upward, compressing the air-fuel mixture into a small space. This compression makes the mixture more explosive.

3. **Power Stroke**: When the piston reaches the top, the spark plug fires, igniting the compressed mixture. This controlled explosion forces the piston downward, generating power.

4. **Exhaust Stroke**: The exhaust valve opens as the piston moves back up, pushing out the burned gases.

These four strokes happen rapidly in each cylinder. A typical car engine has 4, 6, or 8 cylinders working in a synchronized sequence.

The key components that make this possible include:

- **Cylinders**: The chambers where combustion occurs
- **Pistons**: Move up and down within the cylinders
- **Crankshaft**: Converts the pistons' up-and-down motion into rotational motion
- **Valves**: Control the flow of gases in and out of the cylinders
- **Spark plugs**: Provide the spark that ignites the fuel

This rotational energy is then transferred through the transmission to the wheels, propelling your car forward. The entire process repeats continuously while the engine is running, with each cylinder firing in a specific order to ensure smooth operation.
```

### Example 3: Technical Question

**User Query:** "How to rebuild an engine?"

**Classification:**
- Category: Technical Support
- Routed to: Technical Support Staff (deepseek-r1:latest)
- Reasoning: This is a complex technical question requiring detailed procedural knowledge about automotive repair and mechanical expertise.

**Staff Response:**
```
<think>

</think>

Rebuilding an engine is a complex process that requires specialized tools, knowledge, and the right equipment. Here's a general overview of the steps involved in rebuilding an engine:

### 1. **Preparing the Engine**
   - **Drain Coolant**: Flush the engine with antifreeze or water to remove old coolant.
   - **Remove Coolant Level Marker**: Remove the marker to ensure no residual coolant remains inside.
   - **Set the Coolant Level**: Fill the engine with fresh coolant, setting it according to the manufacturer's specifications.

### 2. **Inspecting the Engine**
   - **Visual Inspection**: Check for wear and tear on all moving parts, including the cylinder walls, piston rings, and valve train components.
   - **Engine Knock**: Listen for unusual noises that could indicate engine damage or misalignment.
   - **Timing Chain (If Applicable)**: Check for cracks in the timing chain or tension in the starter belt.

### 3. **Preparing Components**
   - **Replacing Gaskets and Seals**: Replace worn-out gaskets, rubber seals, and packing to ensure proper sealing of moving parts.
   - **Pulling and Inspecting Components**: Remove all engine components (cylinders, valves, piston rods, connecting rods, crankshaft, etc.) for thorough inspection.

### 4. **Rebuilding the Engine**
   - **Cylinder Liners**: Re-boring cylinder walls to smooth them or re-lining with high-quality cylinder liners.
   - **Valves and Valve Train**: Replacing worn-out valves, pushers, and springes, or replacing the entire valve train if necessary.
   - **Piston Rings**: Replacing worn piston rings and ensuring they are properly seated in the engine.

### 5. **Connecting Rods**
   - **Inspecting and Replacing**: Checking for wear or cracks in connecting rods and replacing them as needed.

### 6. **Crankshaft and Bearings**
   - **Checking for Wear**: Inspecting the crankshaft for signs of wear, such as bent pins or damaged journals.
   - **Replacing Bearings**: Replacing worn-out bearings with new ones to ensure smooth operation.

### 7. **Piston and Deck Rings**
   - **Inspecting Pistons**: Checking piston integrity and seating them in the engine properly.
   - **Re-lining Deck Rings**: If necessary, re-lining deck rings to prevent excessive wear.

### 8. **Final Assembly**
   - **Reassembling Components**: Carefully reassembling all engine parts back together, ensuring that everything is properly seated and aligned.
   - **Testing**: Thoroughly testing the rebuilt engine to check for leaks, low fluids, alignment issues, and performance.

### 9. **Coolant Level and Charging**
   - **Setting Coolant Level**: Refilling the engine with fresh coolant according to the manufacturer's specifications.
   - **Charging System (If Applicable)**: Ensuring that the charging system is working properly if applicable.

### 10. **Final Testing**
   - **Performance Test**: Running the rebuilt engine to ensure it performs as expected and meets all factory specifications.

### Important Considerations:
   - **Timing**: Rebuilding an engine can be a time-consuming process, so plan accordingly.
   - **Tools and Skills**: Rebuilding requires specialized tools and mechanical skills. If you're unsure about any step, consider consulting a professional mechanic or online resource for guidance.
   - **Parts Quality**: Ensure that all new parts are of high quality to prevent future issues.

### Safety Precautions:
   - Always work in a well-ventilated area to avoid inhaling any fumes from the engine fluids.
   - Use appropriate PPE, especially if handling hot components or working with compressed air.

Rebuilding an engine is a challenging task that requires patience and attention to detail. It's recommended to only attempt this if you have a good understanding of automotive mechanics and access to the necessary tools and resources.
```

**Final Answer to User:**
```
Rebuilding an engine is a complex task that requires technical knowledge, specialized tools, and careful attention to detail. Here's a comprehensive guide to the process:

## Engine Rebuilding Process

### Preparation and Disassembly
1. **Document everything**: Take photos and notes during disassembly to ensure correct reassembly
2. **Drain all fluids**: Remove oil, coolant, and other fluids
3. **Remove the engine**: Either remove it from the vehicle or work on it in place
4. **Disassemble systematically**: Work methodically, keeping parts organized
5. **Clean components**: Use appropriate cleaning methods for different parts

### Inspection and Assessment
1. **Check for damage**: Inspect all components for wear, cracks, or other damage
2. **Measure critical components**: Use micrometers and other precision tools to check:
   - Cylinder bore dimensions and roundness
   - Crankshaft journal dimensions and condition
   - Piston and ring wear
   - Valve and valve seat condition

### Machine Work
1. **Cylinder boring/honing**: Refinish cylinder walls to proper dimensions
2. **Crankshaft grinding**: Restore journals to factory specifications
3. **Valve job**: Resurface valve seats and valves
4. **Block decking**: Ensure perfect flatness of the cylinder head mating surface

### Component Replacement
1. **Replace worn parts**: Typically includes bearings, seals, gaskets, timing components
2. **Consider upgrading**: Performance parts if desired
3. **Match components**: Ensure proper clearances between related parts

### Reassembly
1. **Follow torque specifications**: Use a torque wrench for all critical fasteners
2. **Apply proper lubrication**: Oil or assembly lube on all moving parts
3. **Check clearances**: Verify proper oil clearances for bearings
4. **Time the engine**: Set valve timing and ignition timing correctly
5. **Seal all connections**: Use appropriate gaskets and sealants

### Break-in and Testing
1. **Prime the oil system**: Before first start
2. **Follow proper break-in procedure**: Specific RPM ranges and varying loads
3. **Check for leaks**: Monitor fluid levels and connections
4. **Tune the engine**: Adjust for optimal performance

## Important Considerations
- This is not a beginner-level project; consider your skill level and available tools
- Factory service manuals are essential references
- Budget for unexpected expenses and replacement parts
- Allow sufficient time; rushing increases the risk of errors

For a successful engine rebuild, patience and precision are critical factors. If you're unsure about your abilities, consulting with or hiring a professional may be worthwhile for certain aspects of the rebuild.
```

## Routing Logic Implementation

The router works by analyzing each incoming query and routing it to the specialized staff based on the following logic:

1. For **general knowledge questions** (definitions, how things work in general, everyday knowledge):
   - Route to: General Knowledge Staff (llama3.1:8b)
   - Example queries: "What is a car engine?", "How does a car engine work?"

2. For **technical questions** (detailed procedures, technical expertise, specialized knowledge):
   - Route to: Technical Support Staff (deepseek-r1:latest)
   - Example queries: "How to rebuild an engine?"

The routing decision is made by analyzing:
- The type of information being requested
- The level of technical detail required
- Whether the question is seeking procedural knowledge or factual information

## Implementation Notes

- The router maintains context between interactions, allowing for follow-up questions
- Each specialized staff has a unique system prompt optimized for their domain
- The router synthesizes and formats responses from the specialized staff before presenting to the user
- Additional specialized staff can be added for other categories as needed

