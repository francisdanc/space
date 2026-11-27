# Space

A space-industry game in the making: forage and mine on a procedural Earth-sized planet and its moon, build modular factories, and trade in a supply-and-demand economy between bases.

A procedural, Earth-sized planet you can fly around, from orbit down to walking height, built in Rust with Bevy 0.16.

## Run

```
cargo run
```

## Controls

You start on foot. **Tab** switches between walking and the free camera; switching back to walking drops you on the ground below the camera.

| Key | On foot | Free camera |
|---|---|---|
| Left click / Esc | Capture / release the mouse | same |
| Mouse | Look | Look |
| W A S D | Walk | Move |
| Shift | Run | Boost (×8) |
| Space | Jump | Up |
| V | Third-person view | – |
| C | – | Down |
| Q / E | – | Roll |
| Scroll wheel | – | Speed multiplier |

**Getting started:** a new game starts at **Haven Outpost** on the planet, beside your ship on the landing pad. The bottom right shows the distances to Haven and to **Selene Station** on the moon (~31,000 km away: climb above 60 km, burn towards the moon with boost, brake with B on the way in). Each base has a glowing beacon you can see from far away.

**Navigation:** markers on screen show both bases, the moon and your factory sites with distances; the selected target (yellow) stays pinned to the screen edge when it's behind you. **N** cycles the target. While piloting, the top line shows distance, closing speed and ETA, and warns BRAKE NOW when it's time to slow down. **T** engages the travel autopilot: it climbs out, routes around the planet if it is in the way, cruises, brakes and hands control back hovering above the destination (Haven to Selene takes about 6 minutes). T again, or any flight key, takes over.

**Mining:** ore deposits are scattered over both worlds by geology: iron and copper everywhere, quartz in deserts and on beaches, titanium high in the mountains, platinum on the highest peaks; on the moon titanium, iron, helium-3 (richest in the dark maria), platinum, and rare void crystals in deep craters. Look at an outcrop and **hold E** to mine it; deposits shrink and run out. Harder ores need better drills: iron, copper and quartz can be mined by hand or with the basic drill; titanium and helium-3 need a **Titanium drill**; platinum and void crystals need a **Plasma drill**. The titanium drill is built from titanium alloy, which you can buy at Selene Station to get started.

**Factories:** **B** enters build mode: **1-8** pick a module (mining drill, conveyor, smelter, storage box, titanium drill, plasma drill, logging station, sawmill), **R** rotates, **E** places (the cost comes out of your inventory), **X** takes a module down (full refund). The first module starts a site; modules near it snap to its 2 m grid. Each module hands its output to the module in front of it (conveyors show an arrow). Drills mine the deposit they stand on; smelters turn iron ore into iron plate, copper ore into copper wire, quartz into silicon and titanium ore into titanium alloy. The first smelter is built from raw ore. Out of build mode, **E** on a module collects its output (or loads ore into a smelter).

**Timber:** a **logging station** fells the trees within 24 m of it, one every 15 s, leaving stumps (trees regrow after 3 hours). Spruce and fir give softwood logs, broadleaf trees hardwood logs, and the giant redwoods redwood logs. A **sawmill** turns them into lumber, hardwood planks and redwood beams. The moon has no trees, so Selene pays well for anything wooden.

**Trading:** walk up to a base's trade terminal and press **E**. Prices follow supply and demand: each base produces and consumes goods at its own rates (Haven grows food and wants fuel and parts; Selene mines titanium, iron and helium-3 but has to import food), selling floods a market and lowers its price, buying drains it, and prices recover over time. Shortages and gluts come and go (see the news). Carry goods between the bases for profit.

**Saving:** **F5** saves, **F9** loads, the game autosaves every minute and loads your save when it starts (`saves/save.json`; delete it, or set `SPACE_NEW_GAME=1`, for a new game).

**Foraging and survival:** look at a berry bush, flowering bush, wild apple tree, mushrooms or a fruiting cactus within reach and press **E** to pick it; it regrows after several minutes, and every harvest can turn up rare finds (golden chanterelles, black truffles, amber). **R** eats the cheapest food you carry, **I** shows the inventory (24 slots, with each item's rarity and the total value in credits). Hunger drains over about 45 minutes (faster when running); starving makes you too weak to run.

In deep water you swim: WASD moves towards where you look (look down to dive), Space rises, C sinks, Shift swims faster. Shallow water slows walking. The ship floats if it lands or parks on the sea.

A ship is parked 26 m in front of you with its ramp down. Walk up the ramp and through the door, go to the pilot seat at the front and press **F** to sit down; **F** again gets you up. You can get up and walk around the cabin while the ship is flying: if it's airborne when you leave the seat, the engines keep it hovering. Walk out of the door to leave (careful if you're in the air).

| Key | Flying the ship |
|---|---|
| Mouse | Pitch / yaw |
| Q / E | Roll |
| W A S D | Thrust forward / back / sideways |
| Space / C | Thrust up / down |
| Shift | Boost (×5 in the atmosphere, ×10 in space) |
| B | Brake: burn against your motion until stopped |
| X | Flight assist on/off |
| V | Cockpit view (default) / chase camera |

**Coming home from orbit:** hold **B** to brake to a stop (about 13 s from orbital speed), then let yourself fall or thrust down; below 60 km flight assist takes over again and you can descend and land with C.

**Flying to orbit:** climb with Space (hold Shift to boost). Above **60 km** flight assist hands over to Newtonian flight: gravity is no longer cancelled, so you need sideways speed to stay up. Burn forward, parallel to the horizon, until you're moving at about 7.8 km/s. The HUD shows your vertical speed, the highest and lowest points of your orbit (apoapsis and periapsis), and **ORBIT** once the lowest point clears the atmosphere (100 km). Gravity weakens with distance (inverse square), so orbits are real Keplerian orbits.

With **flight assist** on (the default), letting go of the controls makes the ship hover in place. Top speed scales with altitude (slow near the ground, up to 30 km/s high up), and descent is slowed automatically near the ground so landings are gentle. With assist off, the ship is pure Newtonian: thrust adds velocity and gravity pulls you down.

In the free camera, speed scales with altitude, so the same keys take you across the planet from orbit or let you move slowly near the ground.

## How it works

| File | What it does |
|---|---|
| `src/noise.rs` | Perlin noise in f64 with analytic derivatives, fBm, ridged multifractal, derivative-damped ("eroded") fBm |
| `src/erosion.rs` | Planet-scale landscape evolution: Priority-Flood routing, drainage area, implicit stream-power incision, hillslope diffusion on a 6x512x512 cube-sphere grid; cached to `cache/` |
| `src/terrain.rs` | Height function (with erosion), climate (`Ecology`), cube-sphere mapping, chunk builder: ground colours from soil, vegetation, snow and shores, plus each chunk's plants |
| `src/life.rs` | A planet's biosphere: flora, fauna, civilization. Earth-like home set and seed-generated alien life |
| `src/flora.rs` | Plant species and climate envelopes, deterministic placement layers, per-chunk plant meshes, plant colliders |
| `src/impostor.rs`, `src/impostor.wgsl` | Tree impostors: every big plant species is rendered on the CPU at startup from 8 azimuths x 4 elevations (albedo, normal, depth); beyond ~450 m each tree is one camera-facing quad blending the 4 nearest views |
| `src/plants.rs` | Procedural plant and rock meshes (3 levels of detail each), Earth-like and alien forms |
| `src/surface_map.rs` | Planet-wide cube map of texture-layer weights (~10 km/texel), baked at startup; the terrain shader uses it far away so biomes and coasts stay sharp from orbit |
| `src/planet.rs` | Quadtree LOD (up to level 18, ~1 m vertex spacing), worker-thread meshing, floating origin |
| `src/camera.rs` | Free camera with an f64 position, ground collision and horizon levelling |
| `src/physics.rs` | Avian physics in f64 world space, per-body terrain collision patches (heightfields) |
| `src/player.rs` | First-person player: capsule body, gravity toward the planet centre, walk/run/jump, Tab mode switch |
| `src/ship.rs` | Ship: mesh and colliders from one parts list, flight assist, walkable interior (interior proxy), pilot seat, cockpit/chase camera |
| `src/atmosphere.rs`, `src/atmosphere.wgsl` | Physically based atmosphere: Rayleigh + Mie single scattering ray-marched per pixel, clipped by the depth prepass (aerial perspective), plus matching sunlight colour and sky light |
| (water) | The sea is a perfect sphere at sea level shaded per pixel in `atmosphere.wgsl`: depth-based colour and transparency from the depth prepass, Fresnel sky reflection (the same atmosphere integrated along the reflected ray), GGX sun glitter, a 12-wave wind-sea spectrum that tiles seamlessly, surf foam on beaches, underwater fog. `physics.rs` `Floats`: buoyancy, drag and righting torque (the ship floats); the player wades, swims and dives (`player.rs`) |
| `src/hydro.rs`, `src/water.wgsl` | Rivers and lakes from the erosion's drainage: river segments along the flow graph (width and depth from catchment, level falling downstream, meanders from a domain warp) and lakes in filled depressions of wet climates. `carve` shapes channels, banks, floodplains and lake basins inside `Terrain::height`; `water` gives the water level and flow. Water surfaces are per-chunk meshes shaded like the sea (shared `atmo_common.wgsl`), with ripples flowing downstream and white water on steep stretches. Riverbanks get riparian moisture, so green corridors follow rivers through dry land |
| `src/items.rs` | Item registry (name, category, rarity tier, stack size, food value, base value in credits) and the slot-based inventory |
| `src/forage.rs` | Foraging (targeting, harvest yields and rare bonus finds, regrowth), hunger and eating, and the survival HUD (prompt, food bar, pickup messages, inventory panel). Picked plants are tracked by placement cell in `FloraContext::harvested`, and their chunks are rebuilt with the bare plant form |
| `src/world.rs`, `src/moon.rs` | The home system's bodies: which body is nearest, local up, gravity (patched conics: inside the moon's sphere of influence only the moon pulls), ground heights; the cratered moon's terrain and its LOD chunks |
| `src/interact.rs` | What the player is looking at: plants, ore, modules and terminals offer themselves with an aim score; the best gets the prompt and the E key |
| `src/mining.rs` | Seeded ore deposits by geology on both bodies, hand mining, depletion |
| `src/factory.rs` | Modular factories: sites, grid, build mode, drill/conveyor/smelter/storage simulation |
| `src/economy.rs` | Haven Outpost and Selene Station (sites, buildings, terminals, beacons) and the supply-and-demand markets, trade screen and navigation |
| `src/save.rs` | Save/load of everything the player changed (F5/F9, autosave, load on start) |
| `src/sky.rs` | Sun, stars, HUD |
| `tools/textures/build_ship_textures.py` | Ship PBR texture sets (painted panels with chipped paint, riveted plate, diamond plate, bare metal, leather) from CC0 ambientCG scans; run it before `tools/blender/ship.py` |
| `src/debug.rs` | Automated test settings (below) |

**Walking inside a moving ship** uses an *interior proxy*: each ship has a static copy of its colliders parked near the planet centre, where nothing else exists. Inside the cabin, the player's physics runs against that copy in ship-local coordinates with artificial gravity, so the ship can fly, turn and accelerate without throwing you around. The camera maps you back onto the real ship every frame, and crossing the doorway converts between world and ship space, keeping velocities.

The camera always stays at (0,0,0). Every object's true position is an f64 `WorldPos`, and each frame it is converted to a camera-relative f32 `Transform`. This keeps f32 rendering precise at planet scale.

## Nature

**Terrain and erosion.** The planet's large-scale relief (continents, ranges, hills) is run through a landscape-evolution simulation on first launch (about 7 s; cached in `cache/` per seed). Water is routed to the sea, drainage areas accumulated, and rivers cut valleys in proportion to their catchment (stream-power law), so mountain ranges get dendritic valley systems and river gorges. Finer gullies come from derivative-damped noise, which smooths slopes and sharpens crests at every scale. The simulation also yields a river "wetness" map that feeds the climate.

**Climate.** Temperature falls with latitude and altitude; moisture comes from noise, coasts and rivers, minus subtropical dry belts. Home-planet land is roughly 31% grassland, 15% tundra, 12% broadleaf forest, 11% taiga, 8.5% giant-conifer rainforest, 3% cactus desert and 19% ice, rock or barren ground (`cargo test --release biome_shares -- --ignored --nocapture`).

**Life.** Each planet has a biosphere generated from its seed:
- **Home planet:** 19 plant species. The giant-conifer rainforest has 70 m redwood-like trees, young firs, sword ferns, fallen logs, mossy boulders and mushrooms. There are also spruce taiga, dwarf pines, tundra shrubs, broadleaf woods, berry and flowering bushes, bracken, grass, wildflowers, cacti, dry shrubs and boulders. There are 5 animal species (described, not yet spawned).
- **Alien planets:** generated species built from alien forms (mushroom trees, crystal spires, coral tendrils, bulb clusters, fan plants), a seed palette for foliage and soil, random climate niches, animals with random body plans, and a 40% chance of an intelligent species with a civilization (tech level, settlement density, habitat).

Plants are placed on global per-layer grids (canopy ~10 m, understory ~5 m, ground ~2.4 m, detail ~1.2 m). Each cell picks at most one species weighted by climate fit, so a given tree is in the same place at every level of detail and in the collision. Big trees are drawn out to ~7 km (thinning to the most prominent), understory to ~900 m, ferns and grass to ~230 m (with a sparse fringe beyond). Tree trunks, logs and boulders are solid; ferns, grass and bushes are not. Sun shadows (2 cascades, 250 m) are cast by trees, bushes, rocks, ships and the body. Measured: ~90 FPS in dense forest with ~15-20 M plant triangles on an RTX 5060 (optimised dev build).

Fauna and settlements are designed to reuse the same climate envelopes and placement grids. Clearings (currently the landing glade) are where settlements will clear the vegetation.

## Test settings (environment variables)

- `SPACE_START=forest` (default: a flat glade in the densest canopy forest), `species:<name>` (where that plant grows densest, e.g. `species:cactus`), `ground|mountain|sunset` (on foot), `orbit|high|low|fly-forest|fly-species:<name>|fly-mountain` (free camera; `fly-` = 120 m up) or `leo` (in the pilot seat, in a 400 km circular orbit): choose where you start
- `SPACE_BIOSPHERE=alien:<seed>` or `barren`: replace the home planet's life with a generated alien biosphere, or none. The biosphere summary is printed at startup.
- `SPACE_AUTOWALK=1` (or `=run`): hold "walk forward" automatically; `run` also holds Shift and turns away from the ship, and the log reports how much of the time you were airborne (bounce check) (the screenshot log reports the player's height above the ground, speed and distance walked)
- `SPACE_SHIPTEST=walkin|fly|flywalk|land`: scripted tests. `walkin` walks up the ramp to the seat; `fly` also sits down, climbs, flies forward and hovers; `flywalk` gets up mid-flight and walks to the back while the ship flies on by itself; `land` lands, gets up and walks out down the ramp; `deorbit` (with `SPACE_START=leo`) brakes, descends and lands; `ascent` flies from the ground to orbit (takes about 3 minutes; logs when it passes 70 km and when it reaches orbit). The screenshot log also prints the ship's current orbit. The screenshot log reports the ship's height above the ground, tilt, speed and the control mode.
- `SPACE_THIRD_PERSON=1`, `SPACE_PITCH=-1.1`: start in third person / looking down
- `SPACE_SCREENSHOT=shots/x.png`: take a screenshot once the terrain has loaded, then exit
- `SPACE_SHOT_AT=seconds`: earliest time to take that screenshot (default 4)
- `SPACE_NOVSYNC=1`: uncap the frame rate, for measuring performance

Tests: `cargo test`. `cargo test --release erosion_map_image -- --ignored` writes `shots/erosion_map.ppm` (shaded relief before and after erosion, plus the river network). To tune the terrain, run `cargo test --release -- --ignored --nocapture`, which prints relief statistics and height profiles.

## Models (Blender)

The ship and the player's astronaut body are built by Blender scripts, so they can be changed and rebuilt in code:

```
# Ship: the game exports its collision parts so the model matches what you walk on.
target/debug/space --dump-ship-parts tools/blender/ship_parts.json
E:/Blender/blender.exe --background --python tools/blender/ship.py -- tools/blender/ship_parts.json assets/models/ship.glb shots/blender

# Astronaut, with Idle / Walk / Run animations.
E:/Blender/blender.exe --background --python tools/blender/astronaut.py -- assets/models/astronaut.glb shots/blender
```

The last argument writes preview renders. If a model file is missing, the game falls back to plain boxes (ship) or no body.

## Performance

Profile with `SPACE_PROFILE=1 SPACE_NOVSYNC=1 SPACE_START=forest SPACE_SCREENSHOT=x.png`: it logs GPU time per render pass, CPU time of the heavy systems and frame-time percentiles. `SPACE_HIDE=big_cards,ssao,...` removes parts to measure their cost.

The game is GPU-bound, and plants are most of the frame. What keeps them affordable:
- **Impostors** for big plants beyond 230-450 m (impostor.rs).
- **Depth-equal leaf shading**: the depth prepass alpha-tests leaf cards; the main pass then tests depth for equality without `discard` (foliage.wgsl, `graphics::depth_equal_main_pass`), so each pixel is lit once instead of once per overlapping leaf layer.
- **Shadows**: 1024 px cascades; only the main leaf sprays of the nearest ring cast shadows, crown fill never does.
- **Atmosphere**: 16 jittered view steps (TAA removes the noise).
- **Streaming**: chunk meshes and their bounds are assembled on worker threads; the main thread hands at most ~3 ms of finished chunks to the renderer per frame, and GPU uploads are capped at 8 MB per frame (`RenderAssetBytesPerFrame`). Leaf-card vertices are 48 bytes (no tangents: the shader derives them).
