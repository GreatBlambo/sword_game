# TODO
1. Map logical image resources to physical ones
2. Render pass roadmap (skip phase 2 and 3 until TODO item 3 is complete):
    - Phase 1: Create physical render pass resource. Temporary implementation is just one render pass with all the passes as subpasses and physical attachments. 
    - Phase 2: Use barriers/semaphores to split into multiple physical render passes to allow compute passes
    - Phase 3: Integrate materials, which may allow more fine grained placement of pipeline barriers
3. Material system