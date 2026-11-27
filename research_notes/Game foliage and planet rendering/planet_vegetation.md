# Procedural vegetation scattering and rendering in planet-scale and large open worlds

Context: Rust/Bevy 0.16 Earth-sized procedural planet. Today, plants are placed deterministically per terrain chunk on CPU worker threads and merged into per-chunk meshes with 3 LOD tiers. Distant forest exists only as a canopy colour on the terrain.

Source quality, read this first. The two best primary sources found were the original slide decks for Horizon Zero Dawn (GDC 2017, Guerrilla) and Serious Sam 4 (GDC 2019, Croteam/AMD GPUOpen). I downloaded both and extracted their text. Most other titles (No Man's Sky, Star Citizen, MSFS, Outerra, Witcher 3/4) have only secondary or forum-level public detail. Their GDC Vault videos are members-only.

---

## 1. How shipped games place and render vegetation

### Takeaway
At large scale every shipped system works the same way. A coarse density/ecotope field, either painted or generated (~1–32 m/texel), feeds a deterministic discretiser: a dither/threshold pattern or a jittered grid. That emits instances near the camera. At distance the trees are replaced by a separate cheap layer: impostor quads, then a "far forest" material splatted into the terrain. Serious Sam 4's 128×128 km world is the closest documented analogue to this project's needs.

### Cited Findings

**Horizon Zero Dawn (Guerrilla, GDC 2017, Jaap van Muijden)**
- All nature is placed procedurally, at run time on the GPU: "500+ asset types, 100.000+ objects in scene, ~250 µs avg busy load on GPU". They started with a traditional CPU system and moved to a GPU-based, real-time, density-based one. — [GDC2017 slides (Guerrilla PDF)](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- Design requirements: "Data driven, Deterministic, Locally stable". Locally stable means changing an input only changes placement locally. — [same](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- An "Ecotope" describes an environment. It determines asset types, asset distribution, colorization, weather, effects, sound and wildlife. — [same](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- WorldData is a collection of 2D maps, streamed in sections, all generated and all paintable, at "~4MB/km²". Examples: Height_Terrain 0.5 m 16-bit; Placement_Trees / Placement_BlockBush / Placement_Undergrowth 1.0 m BC7; Variance_Trees_Bush 2.0 m; Erosion_Flow/Deposition 0.5 m; Topo_Roads/Water/Objects 0.5 m; "RGB Ecotopes A-H" 2.0 m. — [same](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- A single painted Placement_Trees value is decoded into bands: Sparse Trees → Edge Forest → Inner Forest. The Inverse of the tree map drives the "Clearing" undergrowth. Assets carry a footprint value, e.g. Lodgepole Pine 6.0, Douglas Fir 6.0, Grease Wood 3.0, Carex grass 1.0, Fern 1.0. — [same](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- Pipeline per layer: the artist graph is flattened into layers, compiled and merged. Then come DENSITYMAP (single layer, scalable granularity, sampled from mipmapped WorldData), GENERATE (the "discretizing step, dither based, responsible for collision") and PLACEMENT (basis + bounding box, RNG seeded by pattern idx/id). Output is a point cloud copied to CPU memory as Mat3x4. — [same](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- The dither pattern comes from an offline generation tool with two rules: "Even spread thresholds" and "Maximize 2D distance". It has uniform 2D distance w and is "scale[d] to w = footprint". Each point has a threshold, and it spawns when density > threshold. This is effectively a blue-noise point set with ranked thresholds. — [same](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- GENERATE runs one pattern per thread group and one sample per thread. Steps: range test → threshold test → position → normal → stage to buffer. Cost: "~10 µs, VGPR 28". PLACEMENT costs ~7 µs. — [same](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- Collision between species:
  - Different footprints would need "read-back, dependencies" (bad).
  - Same footprint uses "Layered Dithering". The density map stores two values (float2), and each layer does a "two-sided threshold test", so species partition one shared dither pattern and never overlap.
  - Each layer evaluates independently: "more work, but no flushes". They use ordering heuristics, and the pipeline is instantiated 64 times to overlap work on the GPU.
  - [same](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- Staffing: nature assets were created by 3 people and ecotopes by 1 person. — [same](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)

**Serious Sam 4 (Croteam, GDC 2019, "4 Million Acres")**
- Goals:
  - 128×128 km terrain, with real-world vistas of ~50–100 km.
  - "'Background' is not a special case."
  - [GDC 2019 slides (GPUOpen PDF)](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Hybrid procedural approach:
  - Pre-created coarse data at 32 m/sample, 4k×4k for the whole terrain: elevation, "far texture" albedo and **vegetation density**. That totals ~128–200 MB, and it is not even streamed.
  - Finer detail is procedural noise, stored in quadtrees and editable down to 25 cm.
  - Vegetation density "defaults to e.g. 0.28".
  - [same](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Vegetation density, combined with elevation, slope and convexity (the second derivative: hilltops vs valleys), controls materials, grass/flower placement, misc plant distributions and tree probability. "Trees not spawned on steep slopes, or in drylands." Artists "spray-paint" density very roughly to define huge diverse forests. — [same](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Vegetation rendering subsystems, in layers: bare terrain → **forest imposters** → near trees → misc plants & crops → grass blades (real geometry, ~7 tris/blade) → flowers (alpha-keyed). — [same](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Forest placement:
  - **Jittered grid, ~8 m between trees.** 1 km blocks hold ~16,000 trees each.
  - One tile is ~128×128 texels. Each sample stores one tree (offset, rotation, tree type, hue).
  - [same](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Forest imposters:
  - Pregenerated geometry: a grid of 128×128 standalone quads with 2 tris per tree, reshaped from the per-tree texture.
  - They rejected crossed geometry (slower) and plain viewer-facing quads (no parallax). Instead they use 8-direction textures on a viewer-facing quad, with perspective compensation in the pixel shader (cf. "Interior Mapping"), blending the 2 nearest directions.
  - Texture arrays store albedo, subsurface, normal, AO and depth. Depth gives proper shadows.
  - [same](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- **"Far forest: splatted into terrain at distance beyond forest imposters."** Forest-floor materials are separate per region inside forests. — [same](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Grass and flowers:
  - Grass uses pre-batched geometry in grids. Several density grids overlap. The vertex shader samples a splatted mask/colour texture so grass matches the ground material and takes its albedo from the floor.
  - Flowers use pre-batched placeholder geometry (≤32 tris) that the vertex shader reshapes into a real flower by index.
  - [same](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Roads, rivers and patches (fields, lakes, yards) fade out the vegetation map at their transition borders. "Doodads" are added at their edges. — [same](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Terrain: tiles of 33×33 vertices with 512×512 textures. 18 levels / 160 tiles reach 128 km. ~350k tris in a typical scene. — [same](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)

**Ghost Recon Wildlands (Ubisoft)**
- Two systems:
  - A grass system spawns patterns from terrain splatting.
  - A Houdini tool places bushes and trees in a **cascade**: large trees spawn medium trees, which spawn small trees and bushes.
  - Rules cover density, scale, slope, spacing, sun direction, species-per-material and include/exclude masks (flow maps, river beds, cliffs). Rules combine with a delete threshold. Roads, railways, water and paths are excluded automatically.
  - [80.lv](https://80.lv/articles/vegetation-generation-in-ghost-recon-wildlands/)
- Several growth "steps" of the same species, with few species per rule, for naturalness and memory. — [80.lv](https://80.lv/articles/vegetation-generation-in-ghost-recon-wildlands/)
- **LOD distances scale with local vegetation density: denser areas switch to impostors sooner.** Impostors are in-engine-generated dynamic billboards with multiple view angles, plus channels for light direction, fake shadowing and translucency. — [80.lv](https://80.lv/articles/vegetation-generation-in-ghost-recon-wildlands/)

**Far Cry 5 (Ubisoft, GDC 2018, Étienne Carrier)**
- Houdini tools generate biomes, terrain texturing, freshwater networks and cliffs. "Several species will fight for the ground where they can grow and thrive." Grass is oriented on a wind vector map. — [PlayStation Blog](https://blog.playstation.com/2018/03/22/the-procedural-world-generation-of-far-cry-5/); [GDC Vault](https://gdcvault.com/play/1025557/Procedural-World-Generation-of-Far); [SideFX](https://www.sidefx.com/learn/talks/procedural-world-generation-far-cry-5/)
- This is offline (editor-time) generation, unlike HZD's runtime approach. — [GDC Vault abstract](https://gdcvault.com/play/1025557/Procedural-World-Generation-of-Far)

**Outerra (planet-scale engine)**
- Developer Brano Kemen's forum statements: trees were initially billboards, with models planned for close range. "Around 200,000 of such trees rendered in an average forest scene". Maximum density was "~44,000 trees per square km, one tree in one 4.8×4.8 m cell", which implies a grid/cell-based placement. In 2012 the trees did not yet transform into matching billboards at distance. — [Outerra forum via search snippet](https://forum.outerra.com/index.php?topic=15.0) (the page returned 502 when fetched directly, so I could not confirm the quotes in context)

**Star Citizen (Planet Tech v4 / Genesis v5)**
- v4 (from Alpha 3.8) added terrain texture blending, object scattering and biome transitions, with biomes driven by temperature and humidity. Ground and orbital views match, to avoid texture pop. — [theimpound wiki summary](https://theimpound.com/blogs/star-citizen-wiki/planet-tech-v4); [starcitizen.tools](https://starcitizen.tools/Genesis_(Star_Engine))
- Genesis (v5) adds geology, soil type and soil depth data-maps that drive ground texturing and asset scattering. Ground-cover tiles follow the terrain, with blades aligned to gravity. — [starcitizen.tools Genesis](https://starcitizen.tools/Genesis_(Star_Engine)) (community wiki, secondary)

**No Man's Sky**
- The GDC 2017 talk "Continuous World Generation" (Innes McKendrick) covers a pipeline of voxel generation → polygonisation → texturing → population/simulation. I found no public detail on the flora placement algorithm or its density/pop-in handling. — [GDC Vault](https://gdcvault.com/play/1024265/Continuous-World-Generation-in-No); [Game Developer](https://gamedeveloper.com/programming/video-how-continuous-world-generation-works-in-i-no-man-s-sky-i-)

**Microsoft Flight Simulator 2020/2024**
- No official Asobo technical description was found. Community threads report:
  - Tree draw distance is a recurring complaint, tied to the terrain LOD and tree-quality sliders.
  - Asobo reduced tree sizes after early patches.
  - Trees are placed by land-class / "AI" rules, with misplacement over farmland in 2024.
  - Photogrammetry trees are hidden under dedicated 3D tree models in the foreground.
  - [MSFS forum: tree draw distance](https://forums.flightsimulator.com/t/tree-draw-distance-terrible-again/404903?page=4); [MSFS forum: excessive trees 2024](https://forums.flightsimulator.com/t/excessive-amount-of-trees-2024-trees-are-even-more-excessive-than-2020/674238); [AVSIM](https://www.avsim.com/forums/topic/651627-trees-in-msfs-2024/page/4/) (anecdotal, not authoritative)

**Witcher 3 / Witcher 4 (UE5 Nanite Foliage)**
- Witcher 3 is a deep SpeedTree integration. Distant canopies are animated billboards with shared silhouettes. A community mod added higher-resolution billboards with normals and shadowing. — [Wccftech on the tree LOD mod](https://wccftech.com/the-witcher-3-mod-trees-level-of-detail/); [SpeedTree](https://store.speedtree.com/?p=2530)
- UE 5.7 "Nanite Foliage" (Experimental) combines Nanite Assemblies, Nanite Skinning and **Nanite Voxels**. Distant trees become voxels, which also improves distant shadows and lighting. An Epic/NVIDIA demo rendered ~1 million Witcher 4 trees. — [Digital Production](https://digitalproduction.com/2025/11/12/unreal-engine-5-7-foliage-pcg-and-in-editor-ai/); [80.lv](https://80.lv/articles/get-a-glimpse-of-nanite-foliage-with-voxel-representation-in-ue5-7); [Unreal Engine 5.7 release](https://www.unrealengine.com/news/unreal-engine-5-7-is-now-available)

### Inferences
- Two families exist:
  - **Runtime GPU placement from coarse maps** (HZD, SS4's 32 m density + jittered grid, Outerra cells).
  - **Offline authored placement** (Far Cry 5, Wildlands Houdini).
  - A procedural planet can only use the first. The project's CPU-per-chunk deterministic approach is structurally the same as HZD/SS4. What differs is mainly the far-field representation.
- SS4's numbers are a good sanity scale. ~8 m jittered grid ≈ 15.6k trees/km² at full density. Outerra's maximum is 44k/km² (4.8 m cell). HZD keeps ~100k+ objects of all kinds in the scene.

### Gaps
- NMS, Elite Dangerous Odyssey, Starfield, Death Stranding and Sons of the Forest: no primary technical sources found on vegetation placement or distant rendering within the search budget.
- MSFS: no official description of the tree impostor or horizon technique. Only forum anecdotes.
- The Star Citizen v4 "ground cover" specifics (tile size, density) were not found.

---

## 2. Placement algorithms

### Takeaway
The production-proven pattern is a **precomputed, tileable blue-noise point set where each point carries a ranked threshold**. A point spawns if local density > its threshold. That one structure gives density control, determinism, local stability and inter-species spacing (by partitioning threshold ranges). Jittered grids are the cheap alternative (SS4, Outerra). Rule- and cascade-based placement (Wildlands, FC5) adds ecological plausibility.

### Cited Findings
- HZD dither/threshold patterns: even-spread thresholds, maximised 2D distance, scaled so the minimum spacing equals the asset footprint. Species with the same footprint share a pattern via a two-sided threshold test, so there is no read-back. Different footprints would require read-back and dependencies, which they avoided. — [HZD slides](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- SS4 uses a jittered grid with ~8 m spacing and one tree per texel. Tree probability comes from density × slope/elevation/convexity rules. Materials are chosen as `chance[i] = noise(...) * ramp(slope) * ramp(vegetation_density)`, picking the max. — [SS4 slides](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Wildlands uses cascade placement (big trees seed medium and small trees and bushes), spacing rules and delete thresholds between rule layers. — [80.lv](https://80.lv/articles/vegetation-generation-in-ghost-recon-wildlands/)
- Far Cry 5 uses species competition for ground ("fight for the ground"). — [PlayStation Blog](https://blog.playstation.com/2018/03/22/the-procedural-world-generation-of-far-cry-5/)
- A Unity-oriented reimplementation of HZD-style GPU placement exists, but its details could not be fetched (403). — [Medium, Szwajka](https://medium.com/@kacper.szwajka842/gpu-run-time-procedural-placement-on-terrain-cc874e39bbfb); [80.lv guide](https://80.lv/articles/a-comprehensive-guide-on-run-time-procedural-placement-in-unity/)

### Inferences
- These are my recommendations for the project, derived from the above.
- **Build one tile of N blue-noise points, each with a rank threshold in [0,1), offline or at startup.** Use it for every footprint class, scaled per class:
  - In planet space, index tiles by an integer lattice on each cube face (or in chunk-local tangent space) and hash `(face, tx, ty, layer)` to choose among a few rotated or flipped tile variants. This hides repetition and stays deterministic and seed-stable.
  - Because acceptance is `density(x) > threshold(p)`, lowering density only removes points. That gives you **density LOD for free**: at distance, raise the effective threshold to keep only the top-k ranked trees, and the survivors are exactly a subset of the near set. No pop-in from re-randomisation.
- Inter-species spacing:
  - Use HZD-style threshold partitioning for species of the same footprint class. Species A takes `t < dA`, species B takes `dA ≤ t < dA+dB`.
  - Handle different footprint classes hierarchically, Wildlands-style. Place big trees first, then for small layers reject points within radius r of a big tree. That is easy on CPU because the big-tree set per chunk is already known, and it avoids GPU read-back.
- CPU vs GPU: HZD reports ~17 µs of GPU kernel time per layer pass and moved off CPU for iteration speed and scale. Bevy 0.16 compute is viable, but CPU workers already deliver determinism. Keep the CPU path for trees and physics, and consider GPU only for dense, non-collidable ground cover (grass, flowers). SS4 does this with pre-batched grids reshaped in the vertex shader.

### Gaps
- No source gave measured tile sizes or point counts for HZD's dither patterns.
- No primary source on Wang-tile blue-noise use in a shipped game was gathered in this pass.

---

## 3. Distant forest representation and avoiding "bald terrain"

### Takeaway
Shipped large-world games use a **three-layer far field**:
1. Near instanced meshes.
2. Cheap **impostor quads per tree**, placed from the *same* placement data, for kilometres.
3. Beyond that, a **forest colour/material splatted into the terrain**, ideally with canopy height/roughness, AO and normal so it does not look bald.

The research line (Bruneton & Neyret 2012) formalises the far layer as a view- and light-dependent "shader-map" on the terrain, seamlessly blended with near tree representations.

### Cited Findings
- SS4's per-tree impostors:
  - A 128×128 quad grid per 1 km block, 2 tris per tree.
  - 8 directional views with parallax correction, blending the 2 nearest.
  - Albedo, subsurface, normal, AO and depth arrays, with depth used for shadows.
  - Beyond impostor range, "far forest" is splatted into the terrain material.
  - [SS4 slides](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Bruneton & Neyret, "Real-time Realistic Rendering and Lighting of Forests" (CGF 2012):
  - Aerial views show "millions of trees on a wide range of distances, from the camera to the horizon".
  - They use **z-fields** (light-field + height-field per tree) for near trees and a **shader-map** (a location-, view- and light-dependent shader mapped on the terrain that accounts for cumulated sub-pixel effects) for far forest, with a seamless transition between them.
  - Precomputed views use n=9 → 181 views over the hemisphere.
  - A noted limitation is the large z-field data per tree model, which limits model variety and prevents animation.
  - [Eurographics DL](https://diglib.eg.org/handle/10.1111/v31i2pp373-382); [appendix](https://artis.inrialpes.fr/Publications/2011/BN11a/more_details.pdf)
- Wildlands switches to impostors earlier where density is high. — [80.lv](https://80.lv/articles/vegetation-generation-in-ghost-recon-wildlands/)
- Witcher 3 used animated canopy billboards at distance. UE 5.7 Nanite Foliage converts distant trees to voxels, which preserves volume for shadows and lighting at distance. — [Wccftech](https://wccftech.com/the-witcher-3-mod-trees-level-of-detail/); [Digital Production](https://digitalproduction.com/2025/11/12/unreal-engine-5-7-foliage-pcg-and-in-editor-ai/)
- Outerra started with billboards everywhere and added models up close later. — [Outerra forum (snippet)](https://forum.outerra.com/index.php?topic=15.0)

### Inferences
- **The project is missing the middle layer.** Going straight from 3-LOD merged meshes to canopy colour is the classic "bald terrain from altitude" failure. Recommended additions, in order:
  1. **Impostor layer driven by the same deterministic placement.** For each chunk beyond mesh-LOD range, emit one quad (or a point expanded in the vertex shader) per surviving tree, using rank-threshold thinning so only the top-k trees remain at distance. Use an octahedral or 8-direction impostor atlas per species. Draw as a single instanced draw per chunk or per species: Bevy's per-chunk mesh merging still works, since quads are 4 verts.
  2. **Canopy heightfield on terrain** for the far tier. Bake per-terrain-texel forest coverage, canopy height and average canopy colour into the terrain's material channels. In the terrain shader:
     - Use canopy height as a normal and AO perturbation.
     - Add a view-dependent darkening term (gaps between crowns show shadowed floor at grazing angles), which is a cheap approximation of Bruneton's shader-map.
     - Optionally displace the terrain vertex by coverage × canopy height at far LODs, so ridgelines get a forest silhouette instead of a bald one.
  3. **Transitions:**
     - Cross-fade by dithered alpha (screen-door) or by threshold-rank ordering. Trees whose rank exceeds a distance-dependent cutoff fade out first, so the far set is a strict subset.
     - Fade the canopy splat in as impostor density fades out, so the averaged colour stays constant (energy-conserving). Match the impostor's averaged albedo to the splat colour.
- Horizon-scale trees, as in MSFS, are plausibly a combination of land-class coverage maps plus extremely cheap instanced impostors at very low density, with the splat carrying the rest. I found no official confirmation of MSFS's exact method (see Gaps).

### Gaps
- No official MSFS tree-impostor or horizon technique documentation found.
- Decaudin & Neyret volumetric billboards were not retrieved in this pass.
- No measured impostor-range distances were found, except SS4's statement that impostors precede the far splat.

---

## 4. Physics/collision, streaming budgets and instance counts

### Takeaway
Few published hard numbers exist. The ones found are:
- HZD: 100k+ placed objects in scene, ~250 µs average GPU busy time, ~4 MB/km² of world data.
- SS4: ~16k trees per 1 km forest block, 128–200 MB of coarse data for 16,384 km², ~350k terrain tris.
- Outerra: ~200k trees in an average forest scene, max 44k trees/km².

### Cited Findings
- HZD: 500+ asset types, 100,000+ objects in scene, ~250 µs average GPU busy. WorldData ~4 MB/km², streamed in sections. Output point clouds are copied to CPU memory, presumably for gameplay and collision use (the slide shows a "Copy" to CPU). — [HZD slides](https://www.guerrilla-games.com/media/News/Files/GDC2017_VanMuijden_GPUBasedProceduralPlacementInHorizonZeroDawn.pdf)
- SS4: coarse terrain, vegetation density and far albedo total 128–200 MB for 128×128 km and are "not even streamed". Fine detail is generated on the fly. ~16,000 trees per 1 km block. — [SS4 slides](https://gpuopen.com/gdc-presentations/2019/gdc-2019-agtd2-4-million-acres-serious-sam-4.pdf)
- Outerra: ~200,000 trees in an average forest scene, max ~44,000/km². — [Outerra forum (snippet)](https://forum.outerra.com/index.php?topic=15.0)
- Witcher 4 / UE5.7 demo: ~1 million trees using voxel LOD. — [80.lv](https://80.lv/articles/200-trillion-polygons-scene-with-nanite-foliage-voxelization-in-ue5-7); [TechSpot (via search)](https://www.techspot.com/news/111936-nvidia-shows-witcher-4-forest-demo-running-path.html)

### Inferences
- These are my recommendations for the project, derived from the above.
- **Physics:**
  - Create colliders only for trees (and maybe big rocks) within ~50–150 m of the player or ship landing point.
  - Generate them from the same deterministic per-chunk placement list, which is already on CPU. That is a major advantage over GPU placement, which needs read-back.
  - Simple capsule or cylinder trunk colliders suffice. Ground cover gets no colliders.
- **Budgets:**
  - 4 ms render target for vegetation on mid-range hardware.
  - ~1–2k near mesh trees, 10k–100k impostor quads, ground cover in the hundreds of thousands of GPU-instanced blades or cards.
  - These are rough figures in line with the counts above, not sourced.
- **Streaming:**
  - Do placement per chunk on worker threads (already done).
  - Store only the per-chunk placement list (position, rotation, species, scale, rank ≈ 16 bytes/tree). At 15k trees/km², a 1 km² chunk is ~240 KB, which is cheap to cache.
  - Rebuild impostor and mesh batches from it as LOD changes, without re-running placement.

### Gaps
- No primary source gave per-frame instance budgets or collider radii for any of the named games.
- No source on NMS or Elite pop-in mitigation was found.
