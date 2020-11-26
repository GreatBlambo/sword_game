## Render Pass Roadmap
### Phase 1: 
Just one render pass with all the passes as subpasses and physical attachments. 
### Phase 2: 
Use barriers/semaphores to split into multiple physical render passes to allow compute passes
### Phase 3: 
Integrate materials, which may allow more fine grained placement of pipeline barriers

## Materials Roadmap
### Phase 1:
Basic uniforms (ubos, push constants), interface with client code, generating draw calls
### Phase 2:
Textures, storage buffers

## TODO
1. Map logical image resources to physical ones
2. Render pass phase 1
    - Two pass topo sort: one to get topological sorting to merge pass nodes, second to rearrange merged passes which have new dependencies
3. Materials phase 1
4. Load assets, show them on screen
    - Geo only, no textures