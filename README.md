# Render Pass Roadmap
## Phase 1: 
Just one render pass with all the passes as subpasses and physical attachments. 
## Phase 2: 
Use barriers/semaphores to split into multiple physical render passes to allow compute passes
## Phase 3: 
Integrate materials, which may allow more fine grained placement of pipeline barriers

# Materials Roadmap
## Phase 1:
Basic uniforms (ubos, push constants), interface with client code, generating draw calls
## Phase 2:
Textures, storage buffers

# TODO
1. Map logical image resources to physical ones
2. Render pass phase 1
3. Load assets, show them on screen
    - Geo only, no textures
4. Materials phase 1