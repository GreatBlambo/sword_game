## 10/12

### Render Pass ordering detour
Progress on render passes. Made the node dependencies explicit, removed the "sort order" concept and modified it to allow as much overlap between dependent passes as possible.

i.e.:

```
    graphics_passes: {
        gbuffer: {
            color_outputs: [albedo, normal],
            depth_stencil_output: {depth},
            color_inputs: [],
            depth_stencil_input: {},
            ...
        },
        lighting: {
            color_outputs: [color],
            depth_stencil_output: {},
            color_inputs: [albedo, normal],
            depth_stencil_input: {depth},
            ...
        },
        blur_pass: {
            color_outputs: [blur],
            depth_stencil_output: {},
            color_inputs: [color],
            depth_stencil_input: {},
            ...
        },
        blur_pass2: {
            color_outputs: [blur2],
            depth_stencil_output: {},
            color_inputs: [blur],
            depth_stencil_input: {},
            ...
        },
        composite_pass: {
            color_outputs: [backbuffer],
            depth_stencil_output: {},
            color_inputs: [color, blur, blur2, motion_blur],
            depth_stencil_input: {},
            ...
        },
        velocity_pass: {
            color_outputs: [velocity],
            depth_stencil_output: {},
            color_inputs: [],
            depth_stencil_input: {},
            ...
        },
        motion_blur_pass: {
            color_outputs: [motion_blur],
            depth_stencil_output: {},
            color_inputs: [velocity, color],
            depth_stencil_input: {},
            ...
        }
    }
```
Previously, this config would result in this:

```
Pass sorting complete. Result:

Pass name: gbuffer
Pass name: velocity_pass
Pass name: lighting
Pass name: motion_blur_pass <<<<<<
Pass name: blur_pass
Pass name: blur_pass2
Pass name: composite_pass
```
motion_blur_pass has a dependency on velocity_pass and lighting through the velocity and color attachments, respectively. blur_pass and blur_pass2 have a dependency on lighting_pass through the color attachments. All three passes must happen after the lighting pass, but the ordering between them was arbitrary.

This can cause a stall because motion_blur_pass is dependent on the velocity_pass; what if velocity_pass takes much longer than the lighting pass? motion_blur_pass will then block both blur_pass and blur_pass2 until velocity_pass is complete, even though neither has a dependency on it.

Now, motion_blur_pass will be scheduled after both blur passes:
```
Pass sorting complete. Result:

Pass name: gbuffer
Pass name: velocity_pass
Pass name: lighting
Pass name: blur_pass
Pass name: blur_pass2
Pass name: motion_blur_pass <<<<<<
Pass name: composite_pass
```
This is done by replacing the queue in the toposort with a priority queue. The score associated with each pass is the number of already scheduled passes that it _DOESN'T_ have a dependency on. So, because motion_blur_pass has more dependencies, it will be scheduled later than those with less to avoid the latter from waiting on passes they have nothing to do with.

From here, need to work on mapping to physical attachments...I went down the pass dependency rabbit hole today because I needed a structure which will give explicit dependencies between passes, rather than a sort order.