# Tree and Foliage Rendering in Modern Games (visual quality side)

Scope: how AAA games and middleware model and shade trees, leaves, grass and forest floors, with details you can implement in a Rust/Bevy 0.16 (wgpu/WGSL) forward or deferred PBR renderer. The research covered 23 tool calls. Several primary sources could not be fetched (Ben Golus's Medium article returned 403, the SpeedTree docs returned 403, the Guerrilla pages timed out, and the HZD placement PDF was over 10 MB). Where an item comes from background knowledge and not from a fetched source, it is placed under **Inferences** and flagged "(background knowledge, verify)".

---

## 1. Leaf representation: cards, clusters, individual leaves, Nanite micro-geometry; card cutting; atlases

### Takeaway
The proven approach (Crysis 2007 through HZD 2017, RDR2 and most UE4 titles) is alpha-tested **branch-cluster cards**: atlas textures of leaf sprays rendered from 3D, on cards whose outline is cut to the opaque area so little transparent area gets shaded. UE5.7+ "Nanite Foliage" (experimental, 2025–26) replaces cards with **fully modelled leaves, instanced as "assemblies" of parts**. Distant geometry becomes **voxels** whose silhouette holds up, and wind runs through **bones** instead of per-vertex offsets. For a custom wgpu engine, cut-out cards plus a good alpha pipeline are still the realistic target. Unreal's voxel/assembly design is a useful model for far-LOD crowns.

### Cited Findings
- Epic's stated reasons for abandoning cards: alpha masking "causes heavy overdraw and an expensive mask function, and keeps foliage as flat cards". A full triangle representation "hurts cluster culling and distant simplification" and costs a lot of disk space. World Position Offset (WPO) wind adds per-vertex cost, forces conservative cluster bounds, and splits materials into separate raster bins. — [UE Nanite Foliage docs (UE 5.9 page)](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
- **Nanite Assemblies**: reusable parts (branches, fronds) are instanced across one tree, up to 65k part instances per assembly. Part transforms are computed on demand during cluster culling instead of duplicating geometry. One tree went from about 3.5 GB to about 29 MB on disk, and from about 36 MB to about 2.7 MB of streaming memory in one view. On non-Nanite platforms the parts are merged and simplified into a fallback mesh. — [UE Nanite Foliage docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage); [Nanite Assemblies docs](https://dev.epicgames.com/documentation/unreal-engine/nanite-assemblies)
- **Nanite Voxels**: distant clusters are voxelized into 4x4x4 bricks, at most 128 bricks per cluster, which keeps the volumetric silhouette of aggregate geometry like leaves. Without voxels, distant leaves become "sparse, surviving triangles". The switch from triangles to voxels is driven by error, not by a fixed distance. Voxel clusters are binned separately, sorted into depth buckets and rasterized front to back. Each voxel stores a *normal distribution*, and one normal is picked stochastically per pixel when writing the GBuffer; the distribution currently lives in vertex colour alpha. Enable it with Shape Preservation = Voxelize. Status: Experimental. — [UE Nanite Foliage docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
- The UE 5.7 release lists Nanite Foliage as Assemblies + Skinning + Voxels + Dynamic Wind. The Procedural Vegetation Editor (PVE) exports skeletal meshes with wind data. — [UE 5.7 release notes](https://dev.epicgames.com/documentation/unreal-engine/unreal-engine-5-7-release-notes); [UE 5.7 announcement](https://www.unrealengine.com/news/unreal-engine-5-7-is-now-available)
- The Witcher 4 UE 5.6 tech demo (State of Unreal, June 2025) used Nanite Foliage, with each leaf modelled and dynamic wind, and ran at 60 fps with ray tracing on a base PS5. Press reports disagree on whether the feature shipped in 5.6 or 5.7 (HotHardware and IGN say 5.7; Creative Bloq says 5.6 but "some way off"). — [PC Gamer](https://www.pcgamer.com/hardware/the-witcher-4s-leafy-glory-is-all-down-to-epics-nanite-foliage-and-largely-so-is-the-fact-it-can-run-ray-traced-at-60-fps-on-a-lowly-ps5); [HotHardware](https://hothardware.com/news/witcher-4-unreal-engine-5-6-demo); [Creative Bloq](https://www.creativebloq.com/3d/video-game-design/how-the-witcher-4s-infinite-forest-is-being-built-one-branch-at-a-time-using-unreal-engine-5-6)
- Crysis (2007): foliage is rendered double-sided. Leaves use alpha test and grass uses alpha blending. Grass is shaded fully per-vertex to save fill rate. — [GPU Gems 3 ch.16, Sousa (2007)](https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis)
- Horizon Forbidden West (GDC 2022, James McLaren): a deferred-texturing / visibility-buffer-style system was built specifically to speed up foliage and other alpha-tested geometry. — [Guerrilla publication page](https://guerrilla-games.com/read/adventures-with-deferred-texturing-in-horizon-forbidden-west); [GDC Vault](https://gdcvault.com/play/1028035/Adventures-with-Deferred-Texturing-in)
- Horizon Zero Dawn vegetation talk: "Between Tech and Art: The Vegetation of Horizon Zero Dawn" (Gilbert Sanders, GDC 2018). It covers shaders, meshes, render passes and optimizations. The content could not be fetched. — [Game Developer summary](https://gamedeveloper.com/art/video-how-guerrilla-crafted-the-lush-vegetation-of-i-horizon-zero-dawn-i-); [GDC Vault](https://www.gdcvault.com/play/1025530)

### Inferences
- Card-cutting practice (background knowledge, verify): fit the card outline tightly to the alpha with 6–8 vertex polygons instead of quads. SpeedTree, UE4's "Sprite/Card cutout" tool, and the HZD and Far Cry pipelines all do this, because the extra vertices are cheaper than shading and alpha-testing transparent pixels. Unreal's SubUV/particle "cutout" tool works on the same principle. Some studios also use 2–4 intersecting cards per cluster, or cards bent along a V crease, so clusters keep volume from oblique angles.
- Atlas authoring (background knowledge, verify): artists model a branch spray in 3D (SpeedTree, Houdini or photogrammetry) and bake it orthographically into an atlas of 4–16 variants. The bake stores albedo, normal (from the 3D leaves), thickness/translucency, AO and sometimes depth. The cluster cards are then placed on the tree's branch skeleton in SpeedTree. For Bevy, a Blender or Houdini bake to a 2k–4k atlas is enough.
- For a space game with walkable planets, a hybrid is practical: cut-out cluster cards near the camera; at distance, either a crown impostor (octahedral impostor with depth, normal and albedo) or a voxel/"fuzzy crown" representation in the spirit of Nanite Voxels.
- Nanite's per-voxel stochastic normal is a useful idea to borrow even without Nanite. At far LOD, pick a random normal from a stored distribution per pixel and let TAA average it. This preserves the "rough" shading of a crown and avoids flat impostor lighting.

### Gaps
- I could not retrieve the HZD vegetation slides for exact card counts, vertex budgets or overdraw numbers, or primary sources for RDR2, Witcher 3, Kingdom Come Deliverance or Bethesda Creation Engine tree card workflows. Creation Engine uses SpeedTree-derived trees in Skyrim and LOD billboards; this is background knowledge, not sourced.
- I did not find the exact UE5 card "cutout" algorithm documented in a fetched source.

---

## 2. Shading: translucency/subsurface, crown normals, baked AO, specular, hue variation, seasonal colour

### Takeaway
Leaves are shaded with three main ingredients:
- **Two-sided / transmission lighting:** light from behind passes through, tinted by a "subsurface colour" or thickness map, using a wrapped back-face N·L plus a view-dependent forward-scatter lobe.
- **Smoothed "spherical" crown normals:** normals transferred from a hull proxy, so the crown shades as one volume.
- **Baked vertex AO:** stored by crown depth.

Per-instance hue variation is added on top. Crysis's GPU Gems 3 formulas and Unreal's Two Sided Foliage model are directly portable to WGSL.

### Cited Findings
- **Crysis back-side shading** (GPU Gems 3 ch.16, 2007): `fEdotL = saturate(dot(E, -L))^4` (view-through-light lobe). The wrapped back diffuse is `fLdotNBack = saturate(dot(-N, L)*0.6 + 0.4)`. These combine as `vBack = lerp(fEdotL, fLdotNBack, fBackViewDep) * cDiffBackK`. Front: `saturate(N·L)*cDifK + Phong*cSpecK`. The translucency uses −N·L and E·L, times light visibility and an artist-painted subsurface/thickness texture, which can be low-resolution (about 128²). Back shading is multiplied by a back-diffuse map and by the averaged shadow-occlusion factor. — [GPU Gems 3 ch.16](https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis)
- **Crysis ambient**: hemispheric, `cAmbient = lerp(cAmbient*0.5, cAmbient, saturate(N.z*0.5+0.5))`. Final `= (ambient + diffuse + back) * diffuseMap * matColor + spec*glossMap*specColor`, all multiplied by **vertex-alpha AO** (artist-painted or precomputed). Authors' own limitations: leaf specular is isotropic, and SSS is per-vertex, with per-pixel suggested as future work. — [GPU Gems 3 ch.16](https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis)
- **Unreal Two Sided Foliage shading model**: it lets light pass through thin surfaces, and without it "Default Lit appears almost black on the underside". Subsurface Color sets the amount and colour of transmission and is used to mask stems and veins. — [UE shading models docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/shading-models-in-unreal-engine)
- An Epic engineer (2015, UE 4.7) described the model: all lights hitting one side also contribute to the back side, filtered by SubsurfaceColor. The front-face normal is taken into account. Punctual lights get a view-dependent lobe that brightens when looking straight through foliage at the light. Opacity controls self-shadow thickness. In practice the Subsurface Color input is often the base colour, saturated or darkened. — [UE forum "Foliage shading in 4.7"](https://forums.unrealengine.com/t/foliage-shading-in-4-7/17527/10)
- **Crown normals**: build a smooth hull, ellipsoid or sphere proxy around the canopy and transfer its normals to the leaf-card vertices only, not to the trunk or branches. Using a half-sphere or cone stops the underside normals pointing straight down. Pair this with two-sided shading to handle back faces. — [Polycount thread](https://polycount.com/discussion/comment/2490242); [UE forum, spherical normals](https://forums.unrealengine.com/t/spherical-normals-for-trees-blender/98732); [Foliage Prep tool docs](https://superhivemarket.com/products/foliage-prep/docs); [Foliage Normals docs](https://superhivemarket.com/products/foliage-normals/docs)
- **Grass shading (GoT-style)**: normals are tilted outward across the blade width to fake a rounded blade. At distance, normals blend toward the terrain normal to reduce shimmer. Colour = gradient along the blade × clump colour × albedo, plus fake AO from blade length. These come from a reimplementation that attributes them to the Sucker Punch GDC 2021 talk. — [Unity-Grass (Cain Rademan)](https://cainrademan.github.io/Unity-Grass/)
- A secondary summary of the GoT talk says translucency and AO values were output per blade to account for thickness and occlusion. This is unverified. — [Tiger Abrodi blog](https://tigerabrodi.blog/grass-in-ghost-of-tsushima)

### Inferences
- **UE TwoSidedBxDF in WGSL** (background knowledge of UE source, verify against the engine; no public fetched source):
  ```
  wrap = 0.5
  wrapNoL = saturate((-dot(N,L) + wrap) / ((1+wrap)*(1+wrap)))   // energy-conserving wrap (McAuley)
  scatter = D_GGX(a2 = 0.6^4 /*roughness 0.6*/, NoH = saturate(-dot(V,L)))  // forward-scatter lobe
  transmission = lightColor * attenuation * shadow * wrapNoL * scatter * subsurfaceColor
  ```
  In a Bevy deferred setup, store `subsurfaceColor` (or a scalar thickness × albedo) in a spare GBuffer channel and pick the BxDF with a shading-model ID. In Forward+ this is just extra terms in the leaf fragment shader. For transmission, use the shadow map with a depth bias or a "thickness" offset, or use the *opposite face's* shadow, or else leaves self-shadow their own transmission.
- **Hue variation** (background knowledge, verify): SpeedTree and UE's "per-instance random" both lerp albedo toward a "hue variation" colour by a hash of instance position. Seasonal variation: a world-space or biome noise mask blends albedo to autumn colours. Leaves on the same tree should get a per-branch random value from Pivot Painter style data, not only per-instance values.
- **Specular**: leaves are dielectrics (F0 ≈ 0.03–0.04, roughness 0.4–0.6). The waxy upper surface is glossier than the underside. Use specular occlusion from AO to stop interior leaves glowing.
- **Vertex AO by crown depth**: bake `AO = f(distance from crown hull surface toward the centre)` into vertex colour so the interior is darker. This is cheaper than and complements SSAO, which does poorly on alpha-tested leaves.

### Gaps
- No fetched primary source for RDR2, Ghost of Tsushima tree shading, Frostbite or KCD leaf BRDFs. The UE TwoSidedBxDF code is not quoted from a fetched source.
- No sourced specifics on how studios author thickness maps; the Crysis chapter only says "subsurface texture".

---

## 3. Alpha handling: alpha test vs alpha-to-coverage, coverage-preserving mipmaps, TAA, dithered LOD crossfade, sharpened alpha

### Takeaway
Distant foliage thins out ("disappears") because box-filtered alpha mips lose coverage. Fix it offline with **coverage-preserving mip alpha scaling** (Castaño 2010). At runtime, **sharpen the alpha with screen-space derivatives** and use **alpha-to-coverage with MSAA** (Ben Golus), or with TAA use alpha test plus temporal dithering. LOD transitions use a **screen-door dither crossfade** that TAA resolves.

### Cited Findings
- **Castaño, "Computing Alpha Mipmaps" (The Witness blog, Sept 2010)**: coverage = fraction of texels with alpha above the alpha-test reference `A_r`. For each lower mip, bisect a new reference `a_r` in [0,1] so that coverage matches mip 0, then scale the mip's alpha by `A_r / a_r`. For alpha-to-coverage or blending with no fixed reference, pick a reference close to 1. Implemented in NVIDIA Texture Tools as `FloatImage::alphaTestCoverage` and `scaleAlphaToCoverage`, used with a Kaiser mip filter. — [Castaño blog](https://www.ludicon.com/castano/blog/articles/computing-alpha-mipmaps/)
- Crysis used a depth-based post-process edge smoothing pass for alpha-tested vegetation (rotated triangle bilinear samples on detected depth edges). It only works for depth-writing geometry. — [GPU Gems 3 ch.16](https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis)
- Epic names alpha masking as a cost driver (overdraw and mask function cost), which motivates moving to geometric leaves in Nanite Foliage. — [UE Nanite Foliage docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
- Ben Golus, "Anti-aliased Alpha Test: The Esoteric Alpha To Coverage" (Medium, 2017) is the standard reference. Both URLs returned 403, so its content is not quoted here. — [Medium](https://bgolus.medium.com/anti-aliased-alpha-test-the-esoteric-alpha-to-coverage-8b177335ae4f)

### Inferences
- From Golus's article (background knowledge, verify):
  - Sharpened alpha: `alpha = (alpha - cutoff) / max(fwidth(alpha), 1e-4) + 0.5`. This turns the alpha edge into a roughly one-pixel-wide ramp, suitable for A2C with MSAA. It also works for plain alpha test.
  - Runtime mip-alpha correction instead of offline: `alpha *= 1 + max(0, CalcMipLevel(uv*texSize)) * MipScale` with MipScale ≈ 0.25.
  - A2C with no MSAA does nothing. With 4x MSAA it produces 5 levels of dithered coverage.
- In **WGSL/wgpu**: `alpha_to_coverage_enabled` is a field on `wgpu::MultisampleState` and requires `count > 1`. Bevy 0.16 exposes `AlphaMode::AlphaToCoverage` on `StandardMaterial` (needs MSAA). With Bevy's TAA, MSAA is not used, so use `AlphaMode::Mask(cutoff)` plus the fwidth sharpening, plus coverage-preserving mips baked offline (for example NVTT, or your own Rust bisection at asset-build time). `discard` in WGSL disables early-Z, so do a **depth prepass** with the alpha-tested shader and then an opaque `depth_compare = Equal` main pass. Bevy has a `DepthPrepass` component; the main pass then skips overdraw shading. This is how many deferred and forward+ engines make foliage overdraw affordable.
- **Dithered LOD crossfade** (background knowledge, verify): UE's "DitherTemporalAA" and Unity's `LOD_FADE_CROSSFADE` both discard pixels when `fade < bayer4x4(screenPos) (+ temporal offset)`. The incoming LOD uses the complementary pattern, `1 - fade`, so the two never overlap. TAA then averages the dither into a smooth crossfade. Implement it as a per-instance `fade` value in the instance buffer.
- **TAA interaction**: alpha-tested leaf edges plus sub-pixel wind motion cause ghosting and flicker. Write accurate motion vectors that include the wind displacement (previous-frame wind evaluated with the previous time), or TAA will smear leaves. Bevy's prepass motion vectors require the vertex shader to compute the previous position as well.

### Gaps
- I could not fetch Golus's article to quote the exact formulas. The items above are from memory and should be checked against the article.
- No source found for how HZD, RDR2 or GoT specifically combine TAA with alpha-tested foliage.

---

## 4. Wind animation

### Takeaway
There are three generations:
1. **Vertex-colour procedural bending** (Crysis 2007): main bend plus detail bend, with phases from vertex colour.
2. **Hierarchical pivot data** (SpeedTree wind levels; UE Pivot Painter 2, which stores per-element pivot and parent index in textures): each branch rotates around its own pivot and inherits its parent's motion.
3. **Bone or skinned wind** (UE 5.7 Nanite Dynamic Wind): cheaper per vertex and keeps culling bounds tight.

Gusts come from a scrolling noise or wind-field texture sampled per instance or per vertex.

### Cited Findings
- **Crysis main bending**: `bf = pos.z * bendScale; bf = (bf + 1)^2 - 1` (optional shaping; the listing applies `+1` then squares). Then `newPos.xy += wind.xy * bf`, and `pos = normalize(newPos) * originalLength` to keep the length and avoid stretching. Wind is computed per instance on the CPU as the sum of directional and omni wind sources, with attenuation like point lights, per-instance stiffness and decay over time. — [GPU Gems 3 ch.16](https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis)
- **Crysis detail bending**:
  - Vertex colour channels: R = leaf edge stiffness, G = per-leaf phase, B = overall leaf stiffness, A = AO.
  - Object phase is `dot(worldPos, 1)`, and branch phase adds it.
  - Waves: `wavesIn = time + float2(vtxPhase, branchPhase)`, then `waves = (frac(wavesIn.xxyy * float4(1.975, 0.793, 0.375, 0.193)) * 2 - 1) * speed * detailFreq`, then `SmoothTriangleWave`, then `sum = waves.xz + waves.yw`.
  - Displacement: `pos += sum.xxy * float3(edgeAtten * detailAmp * N.xy, branchAtten * branchAmp)`.
  - Helpers: `SmoothCurve(x) = x*x*(3-2x)`, `TriangleWave(x) = abs(frac(x+0.5)*2-1)`. — [GPU Gems 3 ch.16](https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis)
- **SpeedTree wind presets**:
  - Fast/Better/Best/Palm. Better = global sway + branch motion + leaf ripple. Best adds leaf tumble. Palm adds frond ripple.
  - Ripple moves leaf vertices along their normal. Tumble sets how far leaves point into the wind before they oscillate. Twitch adds periodic flips and twists (Throw, Frequency, Sharpness).
  - Frond ripple can optionally recompute normals for lighting, at extra cost. Branch wind is the most expensive component, so enable its second level only where needed.
  - SpeedTree 9+ prefers a "Games" wind mode. — [SpeedTree Modeler: wind-legacy](https://docs.unity3d.com/speedtree-modeler/manual/wind-legacy.html); [wind-games](https://docs.unity.com/en-us/speedtree-modeler/10/wind-games); [SpeedTree Runtime SDK wind shader config](https://docs.unity3d.com/speedtree-runtime-sdk/manual/wind-shader-configuration.html); [Example wind shader](https://docs.unity3d.com/speedtree-runtime-sdk/manual/example-wind-shader.html)
- **Pivot Painter 2**: texture 1 holds the pivot position in RGB and the parent index in A (stored as a float, up to 2048). Texture 2 holds bound lengths and a per-element random value. The element index comes from a UV channel. Epic's sample `PivotPainter2FoliageShader` rebuilds the hierarchy up to 4 levels. — [UE 4.27 Pivot Painter 2 docs](https://docs.unrealengine.com/4.27/AnimatingObjects/PivotPainter/PivotPainter2); [UE5 Pivot Painter 2.0 docs](https://dev.epicgames.com/documentation/unreal-engine/pivot-painter-tool-2.0-in-unreal-engine); [UE PP2 material functions](https://dev.epicgames.com/documentation/en-us/unreal-engine/painter-tool-2.0-material-functions-in-unreal-engine); [Houdini to PP2 tree wind tutorial](https://tianc377.github.io/posts/TreeWindAnimationWithUnrealPivotPainter)
- **UE Nanite Skinning / Dynamic Wind** (experimental):
  - Bone-hierarchy wind replaces WPO and gives tighter bounds plus fixed-function rasterization. Cost is about 0.1 ms GPU per 100k updating bones.
  - Animation stops below a minimum screen size (default 0.1, `r.Skinning.DefaultAnimationMinScreenSize`).
  - Bones are grouped into Simulation Groups with per-group wind influence, imported from JSON.
  - Limitations: global wind direction only, no near-field simulation and no collision. Virtual Shadow Maps need animation disabled at distance. — [UE Nanite Foliage docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage); [Dynamic wind for Nanite foliage](https://dev.epicgames.com/documentation/unreal-engine/dynamic-wind-for-nanite-foliage-in-unreal-engine)
- A user on the UE forums reports bone wind is "much cheaper than WPO as it's done per bone instead of per vert". This is anecdotal. — [UE forum](https://forums.unrealengine.com/t/foliage-in-current-version-and-forward-looking/2685424)
- **Horizon Zero Dawn**: a global wind force field drives the vegetation vertex shaders with three levels of detail of motion. This comes from an unofficial transcript of Sanders' GDC 2018 talk that could not be opened; treat it with caution. — [Game Developer](https://gamedeveloper.com/art/video-how-guerrilla-crafted-the-lush-vegetation-of-i-horizon-zero-dawn-i-)
- **Ghost of Tsushima**: wind was simulated with "vorticles", and the wind also drives the guiding-wind gameplay feature (GDC 2021, "Blowing from the West"). — [GDC Vault](https://gdcvault.com/play/1027350/Blowing-from-the-West-Simulating); [Game Developer](https://gamedeveloper.com/design/using-vorticles-to-simulate-wind-in-i-ghost-of-tsushima-i-)
- **GoT-style grass wind**: scrolling 2D Perlin noise feeds a sin function that modulates the Bezier control points and the blade facing. — [Unity-Grass](https://cainrademan.github.io/Unity-Grass/)

### Inferences
- A Bevy plan with good value for effort:
  - Bake per-vertex data at asset-import time: branch level, pivot position (or an index into a storage buffer of pivots and parents), and phase (hashed).
  - In the vertex shader, apply trunk sway (Crysis main bend with length preservation), then branch rotation about its pivot (Pivot Painter style, 2 levels), then leaf flutter along the normal (Crysis detail bend or SpeedTree ripple).
  - Gusts: sample a world-space scrolling noise texture at the instance origin (one sample per instance) for sway, and at the vertex for flutter.
  - Keep a global wind uniform (direction, strength, gust phase).
  - Compute the previous-frame position the same way for motion vectors.
- For a space game with many planets, a GPU wind field (a 3D texture around the player, the GoT/HZD style) is overkill early on. A global wind direction plus noise is enough.

### Gaps
- The SpeedTree Runtime SDK wind shader math (packed inputs) is not publicly documented in a fetched source; one community thread notes it is poorly documented.
- No sourced details for RDR2, Witcher 3, Far Cry or Frostbite wind systems.

---

## 5. Grass and ground cover; forest floors that do not look flat

### Takeaway
Ghost of Tsushima (GDC 2021, Eric Wohllaib) set the modern standard:
- Blades are generated on the GPU per tile as **Bezier-curve blades**, with per-blade height, width, tilt and bend.
- Voronoi **clumping** groups blades.
- Normals are tilted outward, and blades seen edge-on are widened in view space.
- An LOD drops vertices per blade.
- Shadows are cheap: a raised-terrain dithered depth.

Horizon (GDC 2017) places all small vegetation at runtime on the GPU, using **density maps and dithered discretization in many layers**. Forest floors get their richness from **layered placement rules**: undergrowth density tied to tree proximity and moisture, litter and debris meshes, and terrain colour blending toward the grass colour at distance.

### Cited Findings
- **GoT grass talk** (GDC 2021 Advanced Graphics Summit, Eric Wohllaib): acres of grass are generated within memory and performance limits, and individual blades are rendered and animated. — [GDC Vault](https://gdcvault.com/play/1027033/Advanced-Graphics-Summit-Procedural-Grass)
- Details from a reimplementation attributed to the talk:
  - 15 vertices per blade along a cubic Bezier, with control points from height, width, tilt and bend plus per-blade random variation. The blade tapers.
  - Vertices are concentrated toward the tip, where the bend is.
  - Normals are tilted outward, and blend to the terrain normal at distance.
  - Voronoi clumps with artist parameters: pull-to-centre, point in the same direction, and height, width, tilt and bend.
  - Blade struct: position, rotation, hash, height, width, tilt, bend, normal, colour, wind, side bend.
  - Density falls off with distance, and terrain colour blends toward the grass-top colour at distance to hide the thinning. — [Unity-Grass](https://cainrademan.github.io/Unity-Grass/)
- Blades perpendicular to the camera are stretched horizontally in view space, "as the GDC talk suggested", and blades bend by height. — [GodotGrass (2Retr0)](https://github.com/2Retr0/GodotGrass)
- Secondary and unverified: 15 verts near, 7 verts far. Shadows are cast by raising terrain vertices to grass height and writing dithered depth into the shadow map. About 83k blades and about 2.5 ms are reported, but treat these numbers as unverified. — [Tiger Abrodi blog](https://tigerabrodi.blog/grass-in-ghost-of-tsushima); [jairsantana Godot grass](https://jairsantana.com/posts/grass/)
- **HZD GPU placement** (Jaap van Muijden, GDC 2017):
  - The system is "fully data driven, deterministic and locally stable".
  - The GENERATE step is a dither-based "discretizing step" that is also "responsible for collision". "Layered dithering" means each layer discretizes independently. The pipeline was instantiated 64 times, with each pipeline emitting one layer.
  - Scale: 500+ asset types, more than 100k objects, about 250 µs average GPU busy load. — [Guerrilla slides PDF](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf); [GDC Vault](https://www.gdcvault.com/play/1024120/GPU-Based-Run-Time-Procedural); [80.lv summary](https://80.lv/articles/real-time-procedural-placement-in-horizon-zero-dawn/)
- **Far Cry 5** (GDC 2018, Étienne Carrier): procedural biomes. Grasses reflect terrain humidity and are oriented along a wind vector map. Forests are thicker where soil humidity is protected from direct sunlight. A separate GDC 2018 terrain talk (Jeremy Moore) covers GPU-compute terrain LOD and culling. — [PlayStation Blog](https://blog.playstation.com/2018/03/22/the-procedural-world-generation-of-far-cry-5/); [GDC Vault, procedural world](https://gdcvault.com/play/1025557/Procedural-World-Generation-of-Far); [GDC Vault, terrain](https://gdcvault.com/play/1025480/Terrain-Rendering-in-Far-Cry); [SIGGRAPH 2018 Advances, FC5 open world (with notes)](https://www.advances.realtimerendering.com/s2018/The%20Challenges%20of%20Rendering%20an%20Open%20World%20in%20Far%20Cry%205%20(With%20Notes).pdf)

### Inferences
- **Bevy GoT-style grass pipeline**:
  1. A compute shader per visible terrain tile (for example 16 m) generates blade instances into a storage buffer with an indirect-draw arg buffer. Use a deterministic hash of world position, density from a biome texture, and a Voronoi clump lookup.
  2. A vertex-pulling vertex shader builds the 15/7-vertex Bezier strip from `vertex_index` with no vertex buffer.
  3. The fragment shader applies tilted normals, a base-to-tip gradient, length AO, two-sided translucency (wrap plus a back lobe like the leaves) and gloss.
  4. Shadows: no grass in the shadow pass; use a raised-terrain dithered shadow or screen-space contact shadows.
  Bevy 0.16 supports indirect draws and storage buffers through custom render-graph nodes.
- **Forest floor** (practice, background knowledge, verify): use HZD-style placement layers keyed to "distance to nearest tree" or a canopy-density map:
  - Ferns and saplings cluster near trunks.
  - Litter (leaves, twigs, pinecones) as decals or small meshes.
  - Terrain splat blends to a "forest floor" (duff) material under canopies.
  - Root flare meshes or decals plus a terrain colour or AO darkening ring at tree bases hide where the trunk meets the ground.
  - Grass is suppressed under dense canopy.
  - Darken terrain albedo and AO with the canopy density map, which doubles as a cheap sky-occlusion term.
- Placement rules for a planet: a deterministic hash of the planet-space cell ID keeps placement "locally stable" (HZD's requirement) as LOD and tiles stream.

### Gaps
- Primary GoT slides were not accessed. The specifics are from reimplementations and blogs.
- HZD dithering math (ordered dither pattern, footprint handling) is in the 10 MB+ PDF, which could not be parsed.
- No sources fetched on forest-floor dressing in RDR2, KCD or Witcher 3.

---

## 6. Lighting of forests: canopy shadows, contact shadows, SSAO, specular occlusion, sky occlusion, light shafts

### Takeaway
The fetched sources say little about this. What is sourced:
- Crysis averaged shadow terms to occlude back-lit translucency, and used vertex AO.
- UE's Two Sided Foliage relies on shadow maps with a thickness/opacity control.
- Nanite Foliage notes Virtual Shadow Maps need animation disabled at distance.
- GoT uses cheap dithered depth for grass shadows (secondary source).

Most forest-lighting practice (cascaded shadows with alpha-tested shadow casters, canopy-density sky occlusion, volumetric fog shafts) is standard engine work and is not well documented in public talks specific to foliage.

### Cited Findings
- Crysis: shadow, falloff and projected-texture terms multiply the front diffuse and specular. Shadow terms are averaged over lights into `fOccFactor`, which also multiplies back (translucent) shading, so shadowed leaves don't transmit light. Final colour is multiplied by vertex AO. — [GPU Gems 3 ch.16](https://developer.nvidia.com/gpugems/gpugems3/part-iii-rendering/chapter-16-vegetation-procedural-animation-and-shading-crysis)
- UE Two Sided Foliage: Opacity controls self-shadow thickness for transmission. Ray-traced shadows support foliage transmission when "Transmission" is enabled on the light (2015-era forum answers; may have changed). — [UE forum](https://forums.unrealengine.com/t/foliage-shading-in-4-7/17527/10); [UE forum, RT shadows](https://forums.unrealengine.com/t/foilage-subsurface-scattering-with-raytracing-shadows-issue/154084/5)
- Nanite Foliage: Virtual Shadow Maps require disabling animation at distance, because animated clusters invalidate cached VSM pages. — [UE Nanite Foliage docs](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
- GoT grass: grass shadows come from terrain raised to grass height with dithered depth instead of the full grass pipeline per light (secondary, unverified). — [Tiger Abrodi blog](https://tigerabrodi.blog/grass-in-ghost-of-tsushima)
- Kotaku notes that Horizon Forbidden West's wind visibly modulates light through foliage. This is a non-technical observation. — [Kotaku](https://kotaku.com/horizon-forbidden-west-ps5-wind-guerrilla-games-open-wo-1848555502)

### Inferences
- **Canopy shadows in Bevy**:
  - Render leaf cards into cascaded shadow maps with the same alpha-test shader (Bevy runs a material's prepass/shadow fragment shader for `AlphaMode::Mask`), plus the wind vertex shader so shadows move.
  - Use lower-resolution alpha (higher mip bias) in shadow cascades 2+.
  - Distant cascades can use impostor or proxy-hull casters.
  - Jittered PCF or PCSS softens canopy dapples.
- **Transmission shadowing**: sample the shadow map with an extra normal offset toward −N, or treat the leaf as thin and use the same shadow value, attenuated by `exp(-thickness * k)`. Otherwise the leaf's own shadow kills its back-lighting.
- **Sky occlusion under canopies**: SSAO's world-space radius is too small for a forest. Bake or compute a low-resolution top-down "canopy density" or height map around the player, a cheap sky-visibility term, and multiply ambient and IBL by it. Add vertex crown-depth AO for leaves and SSAO/GTAO for contact detail. Apply specular occlusion from AO; Bevy's environment map light applies AO to specular in 0.16 when SSAO is enabled. Verify that against the Bevy version.
- **Contact shadows**: screen-space ray-marched contact shadows help grass and the bases of ground cover. Bevy 0.16 has `ContactShadows` on lights; check the API.
- **Light shafts**: Bevy's volumetric fog (`VolumetricFog` + `VolumetricLight` on the directional light, available since 0.14) gives god rays through canopy gaps for free once leaves are in the shadow map.
- Specular on dense canopies looks noisy and sparkly. Clamp leaf roughness to ≥ 0.4, and use Toksvig or specular-AA normal-variance filtering on leaf normal maps.

### Gaps
- No primary sources found for RDR2, Witcher 3, Battlefield/Frostbite, KCD or Skyrim forest lighting specifics (for example canopy sky-occlusion techniques, or how many cascades include foliage).
- The SIGGRAPH Advances FC5 open-world PDF was found but not read. It likely contains relevant TOD/GI and occlusion material for forests (FC5 used a top-down "world AO" height-map occlusion, per background knowledge; verify).
- Bevy 0.16 API specifics (ContactShadows, AlphaToCoverage, specular occlusion) are stated from background knowledge and should be verified against the Bevy 0.16 docs and source.
