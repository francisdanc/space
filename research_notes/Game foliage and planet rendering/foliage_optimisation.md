# Rendering Millions of Trees and Foliage: Performance Techniques (for a Rust/Bevy 0.16/wgpu engine)

Context: the target engine currently draws about 30M plant triangles in dense forest at about 60 fps. It uses per-chunk merged meshes with 3 LOD tiers.

Method note: about 18 tool calls were used. Several primary PDFs could not be read. The GDC 2017 Horizon Zero Dawn deck was over 10 MB, the SIGGRAPH 2015 GPU-driven deck had compressed text and no local PDF tools, and the Epic Fortnite Nanite blog returned HTTP 403. Some numbers therefore come from secondary transcriptions, and are flagged where that applies. Items marked "[prior knowledge, unverified]" come from the researcher's background knowledge, were not confirmed in this session, and should be checked before anyone relies on them.

## 1. Impostors (billboards, octahedral and hemi-octahedral, depth/normal atlases, frame blending, switch distance)

### Takeaway
Octahedral or hemi-octahedral impostors are the standard far LOD for trees. They are an atlas of views of the tree captured over a (hemi-)octahedron, rendered on a small convex-hull or box mesh. The three nearest frames are blended, and a depth atlas is used for parallax and per-pixel depth. Each tree then costs a handful of vertices and only a few texture reads. A three.js demo runs 200k trees this way: real-mesh LODs from 15 to 100 units, then impostors from 100 units out. No source found in this session gave a measured ms saving versus mesh LODs.

### Cited Findings
- Ryan Brucks' (Epic) 2018 ShaderBits article "Octahedral Impostors" is the reference most other work cites. It covers blending the three closest captured views for smooth transitions — [meep-engine octahedral impostor README](https://unpkg.com/@woosh/meep-engine@2.124.2/src/engine/graphics/impostors/octahedral/README.md). The ShaderBits page itself returned no readable body text: [shaderbits.com/blog/octahedral-impostors](https://shaderbits.com/blog/octahedral-impostors).
- Octahedral and hemi-octahedral layouts give a fairly uniform spread of view directions with cheap shader math. Polar (pitch/yaw) layouts oversample the poles — [Arycama, Imposter Baking and Rendering](https://arycama.github.io/2024/10/05/Imposters.html).
- Capture works as follows. Each frame index maps to a view vector through the inverse octahedral transform. An orthographic camera is sized to the diameter of the mesh's bounding sphere. The example grid is 12x12 frames of 128x128 — [Arycama](https://arycama.github.io/2024/10/05/Imposters.html).
- A texture array is preferred over an atlas. It has no size or frame-count constraints and avoids bleeding at frame borders and between mip levels — [Arycama](https://arycama.github.io/2024/10/05/Imposters.html).
- Frame blending uses three samples over a triangulated octahedral mesh, which needs fewer texture reads than four-sample quad blending. A single-sample stochastic variant resolved by TAA is possible but causes TAA artifacts — [Arycama](https://arycama.github.io/2024/10/05/Imposters.html).
- Depth is baked to a separate texture. It drives a parallax UV offset, and the pixel shader can write per-pixel depth. That lets the impostor cast and receive shadows and take part in SSAO and contact shadows — [Arycama](https://arycama.github.io/2024/10/05/Imposters.html).
- Use a conservative low-vertex convex hull instead of a camera-facing quad, which causes overdraw. Pick frames from the view vector to the object's centre, not per vertex, otherwise the UVs jumble. Do most of the work in the vertex shader — [Arycama](https://arycama.github.io/2024/10/05/Imposters.html).
- Fitting a convex hull around the union of all sprite frames removed about 30% of pixels with only 5 vertices (Usnul's report; results vary by asset) — [three.js forum, "A forest of octahedral impostors"](https://discourse.threejs.org/t/a-forest-of-octahedral-impostors/85735).
- The three.js forest demo setup:
  - 200k trees on a 3072x3072 terrain.
  - meshoptimizer LODs from 15 to 100 units, octahedral impostors beyond 100 units.
  - A 2048 atlas with 16 sprites per side, hemi-octahedron mode, alphaClamp 0.45, parallaxScale 0.5.
  - Instanced trees with BVH frustum culling.
  - No crossfade or dither at the switch. The argument is that switching once the impostor covers no more screen pixels than its bake resolution hides the pop.

  Source: [three.js forum](https://discourse.threejs.org/t/a-forest-of-octahedral-impostors/85735).
- Usnul's earlier prototype supported variable atlas and grid resolution, mip margins, and baked depth, normal, PBR, albedo and alpha — [three.js forum](https://discourse.threejs.org/t/a-forest-of-octahedral-impostors/85735).
- Houdini's impostor tool bakes full octahedral, "3D", fixed-axis and animated impostors — [SideFX Labs impostor texture](https://www.sidefx.com/docs/houdini/nodes/out/labs--impostor_texture.html).

### Inferences
- For this engine, the biggest structural win is probably to replace the far LOD tier with hemi-octahedral impostors drawn as instances. One shared hull mesh and one atlas or array layer per species, with per-instance position, rotation, scale and species index. Distant chunks would then cost a few vertices per tree instead of merged mesh triangles. Most of the 30M triangles are likely in mid and far chunks.
- Use a screen-size rule for the switch distance: switch when the tree's projected height in pixels is at most the per-frame bake resolution (for example 128–256 px). This is the rule used in the three.js demo.
- Bake albedo+alpha, normal (octahedral-encoded) and depth. Then impostors can be lit by the same sun and PBR path as the meshes and write correct depth for shadows and fog. Hemi-octahedral is enough unless the camera can look up at trees from below, for example from a low-flying ship.

### Gaps
- No measured ms or fps comparison of impostors versus mesh LOD was found. The original ShaderBits article could not be read.
- Brucks' exact recommended frame counts and atlas sizes (often quoted as 8x8 to 16x16 frames on a 2k–4k atlas) were not verified [prior knowledge, unverified].

## 2. LOD (discrete tiers, cross-fade, screen-size selection, HLOD, Nanite and Nanite Foliage)

### Takeaway
Discrete LODs selected by screen size, ending in impostors, remain the norm outside UE5. Nanite handles foliage best when the leaves are opaque geometry rather than alpha-masked cards. Masked materials and WPO (vertex wind animation) are the expensive cases. UE 5.7 adds experimental "Nanite Foliage", which uses voxel LOD for distant foliage, skinned wind instead of WPO, and Nanite Assemblies (instanced parts). One test scene went from 62 to 119 fps.

### Cited Findings
- Epic's Nanite documentation says three things about foliage:
  - Masked materials cost a lot more than opaque ones. Barycentrics are recomputed per pixel during base-pass shading, and the negative space in alpha maps adds overdraw. The fastest results come from not using masked materials at all.
  - Foliage made of geometry "has shown to be faster with Nanite than card approaches."
  - Foliage is the most obvious case for occlusion-culling problems, because a cluster cannot be culled while any part of it is visible, even if its pixels are masked out.

  Source: [Epic docs, Working with Nanite-enabled content](https://dev.epicgames.com/documentation/en-us/unreal-engine/working-with-naniteenabled-content), as summarised by search. The page was not fetched directly.
- WPO changes the vertex logic, so every WPO material is binned into its own raster dispatch, which costs GPU time. One user reported masked Nanite leaves plus WPO made VSM ShadowDepths time jump as the camera got close — [UE forum, Bad performance with Nanite Masked WPO 5.4.2](https://forums.unrealengine.com/t/bad-performance-with-nanite-masked-wpo-in-ue5-4-2/1906424). This is anecdotal.
- UE 5.7 Nanite Foliage (experimental) has these parts:
  - Foliage is built from instanced parts ("Nanite Assemblies") to keep assets small.
  - Wind is skeletal skinning (Dynamic Wind plugin) instead of WPO.
  - "Nanite Voxels" are used for "efficiently rendering foliage geometry in the distance".
  - TSR has thin-geometry detection.
  - Epic says WPO is "generally not suitable for Nanite due to its increased rendering cost".

  No numbers were given — [Tom Looman, UE 5.7 performance highlights](https://tomlooman.com/unreal-engine-5-7-performance-highlights/).
- Early test: 77,376 trees of 20M polygons each went from 62 to 119 fps with Nanite Foliage voxels. This is one user's test, not an Epic benchmark. The tech is used in the Witcher 4 demo — [80.lv](https://80.lv/articles/get-a-glimpse-of-nanite-foliage-with-voxel-representation-in-ue5-7).
- Known 5.7.x issues:
  - The voxel/triangle switch is per cluster. The root cluster must be a single voxel cluster.
  - Voxelised aggregates can look too dense.
  - VSM with voxels is slow without the VSM cache, and Epic-side advice is to use caching for large foliage scenes.
  - Game-thread and tick costs appear at scale.

  Sources: [UE forum, voxelization holes](https://forums.unrealengine.com/t/nanite-voxelization-holes-in-the-mesh/2723578); [UE forum, VSM with Voxelize](https://forums.unrealengine.com/t/ue-5-7-2-vsm-performance-with-nanite-voxelize-and-r-shadow-nanitelodbias/2706943); [UE forum, Megaplants game-thread slowdown](https://forums.unrealengine.com/t/severe-game-thread-slowdown-with-megaplants-nanite-foliage-at-scale-ue-5-7-2/2698484).
- An Epic-affiliated forum post says the voxel path is "very heavily optimized to be an LOD technique", with plain Nanite mostly handling voxels that are sub-pixel — [UE forum, New nanite voxel foliage](https://forums.unrealengine.com/t/new-nanite-voxel-foliage/2585978).

### Inferences
- The UE 5.7 design suggests a direction for the target engine's near and mid tiers: opaque leaf geometry (no alpha test), wind applied as cheap rigid or skinned motion per branch instead of per-vertex noise, and a volumetric or impostor representation in the distance. Nanite's own lesson is that masked micro-geometry is the worst case for both culling and shading.
- Dithered cross-fade (screen-door) between tiers hides pops, but both LODs are drawn during the fade. A hysteresis band plus a screen-size switch keeps the overlap short.

### Gaps
- No verified SpeedTree LOD specifics or HLOD measurements were found in this session.
- Cross-fade costs were not quantified.

## 3. GPU-driven rendering (instancing vs merged meshes, Hi-Z culling, MDI, compute scattering; Bevy 0.16 and wgpu status)

### Takeaway
Shipped GPU-driven pipelines cull instances, then clusters of about 64 triangles, on the GPU against a Hi-Z pyramid. They issue roughly one indirect draw per viewport. The CPU saving is 1–2 orders of magnitude in draw calls, but the GPU saving is modest (under 10% of overall geometry in AC Unity). Horizon Zero Dawn places 100k+ objects of 500+ types procedurally on the GPU in about 250 µs average. Bevy 0.16 already does GPU transform and cull, multi-draw indirect and experimental two-phase occlusion culling, but only on Vulkan. On DX12 and Metal, MDI and GPU culling are off. Bevy's meshlet renderer does not support alpha-masked materials.

### Cited Findings
- From the SIGGRAPH 2015 talk "GPU-Driven Rendering Pipelines" by Haar (Ubisoft Montreal) and Aaltonen (RedLynx) — [official slides](https://advances.realtimerendering.com/s2015/aaltonenhaar_siggraph2015_combined_final_footer_220dpi.pdf). The numbers below come from a third-party transcription, [pdf4pro](https://pdf4pro.com/view/gpu-driven-rendering-pipelines-7a5c90.html), and are secondary:
  - Meshes are split into fixed 64-vertex strip or 64-triangle clusters.
  - Culling stages, in order: coarse multi-draw frustum, instance frustum/occlusion, cluster chunk expansion, cluster frustum/occlusion/backface, index-buffer compaction.
  - Static backface culling removes 10–30% of triangles. Overall GPU culling removes 20–40% (backface plus cluster bounds), and 30–80% of shadow triangles.
  - The overall geometry gain is small, under 10%.
  - CPU draw calls drop by 1–2 orders of magnitude.
  - AC Unity has about 10x the instances of the previous AC at about 75% of its CPU cost.
  - Hi-Z timings on PS4:

    | Step | Time |
    |---|---|
    | Pre-pass of the 300 best occluders | ~600 µs |
    | Downsample to 512x256 | 100 µs |
    | Reproject last frame's depth | 50 µs |
    | Build depth hierarchy | 50 µs |
    | Per shadow cascade: camera-depth reprojection | ~70 µs |
    | Per shadow cascade: depth hierarchy | 30 µs |

  - Tangent frame is a 32-bit quaternion, and there is per-cluster triangle visibility of 64 bits.
- Aaltonen's follow-up: multi-draw indirect gives higher performance for high-polygon objects, while custom vertex-shader clustering was better (on GPUs of that time) for small low-poly objects — [Beyond3D thread](https://forum.beyond3d.com/threads/gpu-driven-rendering-siggraph-2015-follow-up.57240) (via search summary).
- Horizon Zero Dawn GPU placement (GDC 2017, van Muijden):
  - 500+ asset types and 100k+ objects.
  - Average GPU load about 250 µs.
  - Placement runs as 64 pipeline instances, one per layer.
  - World data is paintable 2D maps streamed at about 4 MB/km², about 32 bits per m².

  Source: [Guerrilla GDC2017 slides (PDF)](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf), [GDC Vault](https://www.gdcvault.com/play/1024120/GPU-Based-Run-Time-Procedural). These figures come from a search summary because the PDF was too large to fetch.
- Bevy 0.16 GPU-driven rendering ([Bevy 0.16 release notes](https://bevy.org/news/bevy-0-16/)):
  - How it works: the CPU uploads one transform buffer and lists of object IDs, then issues about one draw per list (usually 15 or fewer). The GPU does transform and cull.
  - Techniques: MDI and multi-draw indirect count (MDIC), a mesh allocator that packs meshes into shared buffers, bindless materials, and a retained render world.
  - Gains: 3x or more lower CPU render overhead for scenes with tens of thousands of objects. The Caldera Hotel "heavy" section went from 33.55 ms to 10.16 ms on a mobile RTX 4090 under Vulkan/Linux.
  - Platform support: MDI and GPU culling work only on Vulkan (Windows and Linux). DX12, Metal, OpenGL and WebGPU get GPU transform only.
  - Materials with custom WGSL stay CPU-driven unless they opt in with `#[bindless]`. Morph-target meshes stay CPU-driven.
  - Entities that share a mesh and material are auto-instanced. `MeshTag(u32)` adds per-instance data without breaking instancing; read it with `mesh_functions::get_tag(instance_index)`.
- Bevy 0.16 occlusion culling is experimental two-phase culling, enabled by putting `DepthPrepass` + `OcclusionCulling` on the camera. It has precision issues that can wrongly cull visible meshes, and it can be slower in small or simple scenes — [Bevy 0.16 release notes](https://bevy.org/news/bevy-0-16/).
- Bevy meshlets (virtual geometry, `meshlet` feature) do not support transparent, alpha-masked or transmissive materials — [Bevy MeshletMesh docs](https://dev-docs.bevyengine.org/bevy/pbr/experimental/meshlet/struct.MeshletMesh.html).
  - In 0.16 they use 128-triangle clusters (METIS partitions of `tris.div_ceil(126)`) and rasterise with R64Uint/R32Uint storage-texture atomics.
  - The author calls culling "the biggest bottleneck", and BVH culling was still planned at that point.
  - Texture atomics saved about 0.42 ms in a simple demo.

  Source: [jms55, Virtual Geometry in Bevy 0.16](https://jms55.github.io/posts/2025-03-27-virtual-geometry-bevy-0-16). Later Bevy versions have a "meshlet BVH culling" release note — [Bevy release-content](https://cocalc.com/github/bevyengine/bevy/blob/main/release-content/release-notes/meshlet-bvh-culling.md).
- wgpu feature support ([wgpu Features docs](https://docs.rs/wgpu/latest/wgpu/struct.Features.html)):

  | Feature | Backends | Notes |
  |---|---|---|
  | `MULTI_DRAW_INDIRECT_COUNT` | DX12, Vulkan 1.2+ | Native only. Without the feature, multi-draw appears to be emulated as a series of `draw_indirect` calls (inferred from the docs wording). |
  | `INDIRECT_FIRST_INSTANCE` | Vulkan, DX12, Metal, GL 4.2+ with ARB_shader_draw_parameters, WebGPU | |
  | `SHADER_INT64_ATOMIC_*` | Vulkan, DX12 SM6.6, partly Metal | |
  | `EXPERIMENTAL_MESH_SHADER` | Vulkan, DX12, Metal | Naga compiles it only for Vulkan. |

### Inferences
- The engine's per-chunk merged meshes probably cost few draw calls already. Their weakness is granularity: a whole chunk is culled or drawn as one unit, and every tree in a tier gets the same LOD. Moving to shared per-species templates plus GPU-culled instances (compute pass writing indirect args, one MDI per species × LOD) would allow per-tree LOD, impostor selection and Hi-Z culling. Memory would drop too, because templates replace duplicated geometry.
- In Bevy 0.16, the cheapest route is to spawn trees as entities sharing mesh and material handles (auto-instanced, GPU-culled on Vulkan). Per-instance variation goes in `MeshTag`. For millions of instances, ECS entity overhead may dominate. A custom render node with a compute cull writing `DrawIndexedIndirectArgs` and `multi_draw_indexed_indirect(_count)` is then the scalable option. It needs a wgpu Vulkan or DX12 backend.
- Bevy's built-in occlusion culling is per mesh entity, so it cannot cull parts of a large merged chunk. Forests are also poor occluders of themselves (leaf gaps), so expect smaller Hi-Z gains than in cities, except behind terrain. The terrain itself is the main occluder worth using.
- Bevy meshlets are not suitable for alpha-masked leaves. They would only work with fully opaque leaf geometry and no vertex wind.

### Gaps
- No measured instance-versus-merged-mesh comparison for foliage was found.
- The exact Bevy 0.16 automatic-batching overhead per entity at millions of entities was not found.
- Bevy issue-tracker details on occlusion culling were not checked.

## 4. Overdraw and alpha-test cost (prepass, tight cards, alpha-to-coverage, micro-triangles)

### Takeaway
Alpha-tested foliage is expensive for three reasons: pixels are shaded and then discarded, discard disables early-Z, and empty card area is pure overdraw. Remedies: tight convex-hull cards (about 30% fewer pixels with 5 vertices), a depth prepass so the colour pass runs with an equal depth test, and converting near leaves to opaque geometry. Nanite data shows that masked micro-geometry is the worst case.

### Cited Findings
- Masked materials add overdraw from the negative space in alpha maps and cost extra in shading. Epic recommends avoiding them in Nanite — [Epic Nanite docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/working-with-naniteenabled-content) (via search summary).
- Clusters that are partly visible cannot be occlusion culled, and masked materials make this worse because the masked-out pixels still count as visible — [UE forum, Nanite foliage for open world](https://forums.unrealengine.com/t/nanite-foliage-for-open-world/1886164/10) (via search summary).
- A tight hull around impostor or card silhouettes removed about 30% of pixels with 5 vertices — [three.js forum](https://discourse.threejs.org/t/a-forest-of-octahedral-impostors/85735).
- Camera-aligned quads cause impostor overdraw, and a convex hull is preferred — [Arycama](https://arycama.github.io/2024/10/05/Imposters.html).
- Bevy 0.16 notes that the depth prepass already lets the main pass skip hidden fragments, but vertex and depth-test cost remains. Occlusion culling removes that remaining cost for whole meshes — [Bevy 0.16 release notes](https://bevy.org/news/bevy-0-16/).

### Inferences
- Masked foliage in Bevy: use `DepthPrepass` so the prepass runs the alpha-tested shader once, writing depth only. The main PBR pass then runs on surviving pixels only. Keep the prepass fragment shader minimal (one alpha texture fetch).
- Micro-triangles: GPUs shade in 2x2 quads, so triangles smaller than a few pixels waste up to 75% of fragment invocations [prior knowledge, unverified]. A merged far chunk with 30M triangles is likely dominated by sub-pixel triangles. This is the strongest argument for impostors or very coarse far LODs.

### Gaps
- No measured alpha-to-coverage cost or quad-overdraw figures from a shipped game were found in this session.

## 5. Shadows for foliage

### Takeaway
GPU-driven culling against a reprojected camera depth removes 30–80% of shadow triangles (AC Unity). In UE5, foliage shadows are a known hotspot: VSM caching is strongly recommended for large Nanite foliage scenes, and WPO plus masking blows up shadow-depth cost.

### Cited Findings
- AC Unity / GPU-driven pipeline: shadow-cascade culling uses camera depth reprojected into each cascade (about 70 µs plus 30 µs for the hierarchy per cascade), and culls 30–80% of shadow triangles — [pdf4pro transcription](https://pdf4pro.com/view/gpu-driven-rendering-pipelines-7a5c90.html) of [the SIGGRAPH 2015 slides](https://advances.realtimerendering.com/s2015/aaltonenhaar_siggraph2015_combined_final_footer_220dpi.pdf).
- UE 5.7.2: VSM with voxelised foliage is slow when the cache is disabled, so caching is advised for large foliage scenes — [UE forum](https://forums.unrealengine.com/t/ue-5-7-2-vsm-performance-with-nanite-voxelize-and-r-shadow-nanitelodbias/2706943).
- Masked Nanite leaves combined with WPO caused a large rise in VSM ShadowDepths time near the camera (anecdotal) — [UE forum](https://forums.unrealengine.com/t/bad-performance-with-nanite-masked-wpo-in-ue5-4-2/1906424).
- UE 5.7 turned on VSM receiver masks by default for directional lights. They cost about 10 MB and give "often fairly significant" gains in uncached cases with lots of dynamic geometry — [Tom Looman](https://tomlooman.com/unreal-engine-5-7-performance-highlights/).
- Impostors that write per-pixel depth can cast and receive shadows and take part in contact shadows and SSAO — [Arycama](https://arycama.github.io/2024/10/05/Imposters.html).

### Inferences
- For the target engine:
  - Render the far shadow cascades from the impostor or lowest LOD, with a cheaper alpha-only shader.
  - Cap foliage shadow distance.
  - Update the far cascades at a lower rate (cache them, or update every N frames) when the sun moves slowly.
  - Skip wind animation in distant cascades.

  Each shadow cascade re-draws the geometry, so 30M triangles × 4 cascades is the likely hidden cost.

### Gaps
- No shipped-game ms budgets for foliage shadows were found. Capsule and proxy shadow numbers for trees were not found.

## 6. Memory and bandwidth (vertex compression, shared templates vs merged geometry)

### Takeaway
Shipped pipelines compress heavily: a 32-bit quaternion tangent frame, 16+16-bit UVs, and HZD placement data at about 32 bits per m². Instancing shared templates avoids storing every tree's vertices, which merged chunks do.

### Cited Findings
- AC Unity / RedLynx (secondary transcription): 32-bit quaternion tangent frame, 16+16-bit deferred UVs (8x8 subpixel precision), and a 64-bit visibility mask per cluster — [pdf4pro](https://pdf4pro.com/view/gpu-driven-rendering-pipelines-7a5c90.html).
- HZD: world placement data at about 4 MB/km² (about 32 bits per m²). Instances are generated on the GPU at runtime instead of stored — [Guerrilla GDC2017](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf) (via search summary).
- Bevy 0.16's mesh allocator packs many meshes into shared GPU buffers to enable MDI — [Bevy 0.16 release notes](https://bevy.org/news/bevy-0-16/).

### Inferences
- Merged chunk meshes duplicate template geometry for every tree, so 30M triangles is roughly hundreds of MB of vertex data. Templates plus a 16–32 byte instance record per tree cut this by orders of magnitude, and make GPU-side scattering (HZD-style) possible.
- A possible vertex format, roughly halving a 32–48 byte vertex: half-float or quantised position relative to the template bounds, octahedral normal in 2×8 or 2×16 bits, 16-bit UVs [general practice, unverified].

### Gaps
- No measured bandwidth-saving numbers for foliage vertex compression were found.

## 7. Concrete numbers from shipped games and demos (summary)

### Takeaway
The verified figures are scattered. Foliage placement in HZD costs about 0.25 ms of GPU time on average. AC Unity drew about 10x the instances of its predecessor at 75% of the CPU cost. Bevy 0.16 GPU-driven rendering gave about 3x on Caldera. The Nanite voxel foliage test gave 62 to 119 fps. Ghost of Tsushima grass is about 83k blades at about 2.5 ms, which is unverified.

### Cited Findings
- HZD: 100k+ objects, 500+ types, about 250 µs average GPU placement — [Guerrilla](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf) (via search summary).
- AC Unity: 10x instances at about 75% CPU, draw calls reduced 1–2 orders of magnitude — [pdf4pro](https://pdf4pro.com/view/gpu-driven-rendering-pipelines-7a5c90.html).
- Bevy 0.16: Caldera Hotel heavy section from 33.55 ms to 10.16 ms (mobile RTX 4090, Vulkan) — [Bevy 0.16](https://bevy.org/news/bevy-0-16/).
- UE 5.7 Nanite Foliage: 77,376 trees × 20M polygons, 62 to 119 fps — [80.lv](https://80.lv/articles/get-a-glimpse-of-nanite-foliage-with-voxel-representation-in-ue5-7).
- three.js: 200k trees with octahedral impostors beyond 100 units — [three.js forum](https://discourse.threejs.org/t/a-forest-of-octahedral-impostors/85735).
- Ghost of Tsushima: about 83,000 grass blades at about 2.5 ms per frame. This comes from an uncited secondary blog, so treat it as unverified. Blades are placed per tile in compute and culled by frustum and distance — [Hashnode](https://hashnode.com/posts/what-we-can-learn-from-grass-in-ghost-of-tsushima-renders/69e130b2ffbb787634eb9ef4); talk: [GDC Vault, Procedural Grass in Ghost of Tsushima](https://gdcvault.com/play/1027214/contactUs).

### Inferences
- The engine's current 30M plant triangles at 60 fps is already a heavy load. Typical shipped titles put the vegetation geometry budget in the low millions of visible triangles and rely on impostors past about 100–200 m [prior knowledge, unverified]. The most likely large wins, in order:
  1. Far tier to instanced impostors.
  2. Shadow passes using the lowest LOD or impostors.
  3. Per-tree LOD through instancing with a GPU cull instead of per-chunk merged LODs.
  4. Opaque leaf geometry or tight cards to cut overdraw.

### Gaps
- Far Cry 5, Ghost Recon Wildlands, SpeedTree and Witcher 4 vegetation budgets were not retrieved in this session.
- The Karis 2021 Nanite deep-dive numbers were not retrieved.
- The Fortnite Chapter 4 Nanite blog, which has the tree and WPO specifics, returned 403: [unrealengine.com tech blog](https://www.unrealengine.com/tech-blog/bringing-nanite-to-fortnite-battle-royale-in-chapter-4).
