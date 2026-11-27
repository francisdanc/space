//! Items: everything the player can gather, carry, trade, process or build with.
//!
//! Every item has a rarity tier and a base value in credits; planetary bases will buy and sell
//! around these values, and later factories turn common inputs into rarer, more valuable
//! outputs. Definitions are a static table indexed by `ItemId`.

use bevy::prelude::*;

/// How rare (and valuable) an item is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Exotic,
    Legendary,
}

impl Rarity {
    pub fn name(self) -> &'static str {
        match self {
            Rarity::Common => "Common",
            Rarity::Uncommon => "Uncommon",
            Rarity::Rare => "Rare",
            Rarity::Exotic => "Exotic",
            Rarity::Legendary => "Legendary",
        }
    }

    /// UI colour for the tier.
    pub fn color(self) -> Color {
        match self {
            Rarity::Common => Color::srgb(0.85, 0.85, 0.85),
            Rarity::Uncommon => Color::srgb(0.35, 0.9, 0.4),
            Rarity::Rare => Color::srgb(0.35, 0.6, 1.0),
            Rarity::Exotic => Color::srgb(0.75, 0.4, 1.0),
            Rarity::Legendary => Color::srgb(1.0, 0.7, 0.2),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    /// Edible: restores hunger.
    Food,
    /// Plant material (herbs, resin, fibre) for crafting and trade.
    Botanical,
    /// Mined from the ground (ores, crystals).
    Mineral,
    /// Made in factories.
    Component,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ItemId(pub u16);

pub struct ItemDef {
    pub name: &'static str,
    pub category: Category,
    pub rarity: Rarity,
    /// Most per inventory slot.
    pub stack: u32,
    /// Hunger restored when eaten (0 = not edible).
    pub food: f32,
    /// Base price in credits.
    pub value: u32,
    pub description: &'static str,
}

/// Item ids (indices into `ITEMS`).
pub mod item {
    use super::ItemId;
    pub const BERRIES: ItemId = ItemId(0);
    pub const APPLE: ItemId = ItemId(1);
    pub const MUSHROOM: ItemId = ItemId(2);
    pub const CACTUS_FRUIT: ItemId = ItemId(3);
    pub const CHANTERELLE: ItemId = ItemId(4);
    pub const HERBS: ItemId = ItemId(5);
    pub const TRUFFLE: ItemId = ItemId(6);
    pub const AMBER: ItemId = ItemId(7);
    pub const IRON_ORE: ItemId = ItemId(8);
    pub const COPPER_ORE: ItemId = ItemId(9);
    pub const QUARTZ: ItemId = ItemId(10);
    pub const TITANIUM_ORE: ItemId = ItemId(11);
    pub const PLATINUM_ORE: ItemId = ItemId(12);
    pub const HELIUM3: ItemId = ItemId(13);
    pub const VOID_CRYSTAL: ItemId = ItemId(14);
    pub const IRON_PLATE: ItemId = ItemId(15);
    pub const COPPER_WIRE: ItemId = ItemId(16);
    pub const SILICON: ItemId = ItemId(17);
    pub const TITANIUM_ALLOY: ItemId = ItemId(18);
    pub const SOFTWOOD_LOG: ItemId = ItemId(19);
    pub const HARDWOOD_LOG: ItemId = ItemId(20);
    pub const REDWOOD_LOG: ItemId = ItemId(21);
    pub const LUMBER: ItemId = ItemId(22);
    pub const HARDWOOD_PLANKS: ItemId = ItemId(23);
    pub const REDWOOD_BEAMS: ItemId = ItemId(24);
}

pub const ITEMS: &[ItemDef] = &[
    ItemDef { name: "Wild berries", category: Category::Food, rarity: Rarity::Common, stack: 60, food: 3.0, value: 2, description: "Sweet and a little tart. A handful keeps you going." },
    ItemDef { name: "Wild apple", category: Category::Food, rarity: Rarity::Common, stack: 30, food: 8.0, value: 4, description: "Small, crisp apples from wild trees in temperate woods." },
    ItemDef { name: "Forest mushroom", category: Category::Food, rarity: Rarity::Common, stack: 40, food: 4.0, value: 3, description: "Brown-capped mushrooms from damp forest floors." },
    ItemDef { name: "Cactus fruit", category: Category::Food, rarity: Rarity::Uncommon, stack: 30, food: 10.0, value: 9, description: "Juicy fruit from desert cacti. Mind the spines." },
    ItemDef { name: "Golden chanterelle", category: Category::Food, rarity: Rarity::Rare, stack: 20, food: 12.0, value: 45, description: "A prized mushroom, sometimes found among common ones." },
    ItemDef { name: "Medicinal herbs", category: Category::Botanical, rarity: Rarity::Uncommon, stack: 40, food: 0.0, value: 12, description: "Flowers and leaves with healing properties. Valued by bases." },
    ItemDef { name: "Black truffle", category: Category::Food, rarity: Rarity::Exotic, stack: 10, food: 15.0, value: 220, description: "Hidden under old forests. Worth a fortune to the right buyer." },
    ItemDef { name: "Amber resin", category: Category::Botanical, rarity: Rarity::Rare, stack: 20, food: 0.0, value: 70, description: "Hardened tree resin, occasionally found on old fruit trees." },
    ItemDef { name: "Iron ore", category: Category::Mineral, rarity: Rarity::Common, stack: 50, food: 0.0, value: 3, description: "Rust-red ore, found almost everywhere. Smelts into iron plate." },
    ItemDef { name: "Copper ore", category: Category::Mineral, rarity: Rarity::Common, stack: 50, food: 0.0, value: 4, description: "Green-streaked ore. Smelts into copper wire." },
    ItemDef { name: "Quartz", category: Category::Mineral, rarity: Rarity::Uncommon, stack: 50, food: 0.0, value: 10, description: "Clear crystals, common in deserts and on beaches. Refines into silicon." },
    ItemDef { name: "Titanium ore", category: Category::Mineral, rarity: Rarity::Rare, stack: 40, food: 0.0, value: 28, description: "Found high in the mountains, and widely on the moon." },
    ItemDef { name: "Platinum ore", category: Category::Mineral, rarity: Rarity::Exotic, stack: 20, food: 0.0, value: 140, description: "Rare veins on the highest, steepest peaks." },
    ItemDef { name: "Helium-3", category: Category::Mineral, rarity: Rarity::Rare, stack: 30, food: 0.0, value: 65, description: "Fusion fuel trapped in lunar regolith. Planetside reactors pay well for it." },
    ItemDef { name: "Void crystal", category: Category::Mineral, rarity: Rarity::Legendary, stack: 10, food: 0.0, value: 900, description: "A crystal that hums faintly. Found only in the deepest lunar craters." },
    ItemDef { name: "Iron plate", category: Category::Component, rarity: Rarity::Common, stack: 50, food: 0.0, value: 9, description: "The basic building material of every factory." },
    ItemDef { name: "Copper wire", category: Category::Component, rarity: Rarity::Common, stack: 50, food: 0.0, value: 11, description: "Wiring for machines and electronics." },
    ItemDef { name: "Silicon", category: Category::Component, rarity: Rarity::Uncommon, stack: 40, food: 0.0, value: 30, description: "Refined from quartz. Electronics and solar cells need it." },
    ItemDef { name: "Titanium alloy", category: Category::Component, rarity: Rarity::Rare, stack: 30, food: 0.0, value: 80, description: "Light, strong alloy for ships and stations." },
    ItemDef { name: "Softwood log", category: Category::Botanical, rarity: Rarity::Common, stack: 30, food: 0.0, value: 5, description: "Spruce, fir and pine. The everyday timber." },
    ItemDef { name: "Hardwood log", category: Category::Botanical, rarity: Rarity::Uncommon, stack: 30, food: 0.0, value: 12, description: "Dense broadleaf timber for furniture and flooring." },
    ItemDef { name: "Redwood log", category: Category::Botanical, rarity: Rarity::Rare, stack: 20, food: 0.0, value: 40, description: "From the giant conifers: huge, straight and rot-resistant." },
    ItemDef { name: "Lumber", category: Category::Component, rarity: Rarity::Common, stack: 60, food: 0.0, value: 4, description: "Sawn softwood boards. Every settlement builds with it." },
    ItemDef { name: "Hardwood planks", category: Category::Component, rarity: Rarity::Uncommon, stack: 40, food: 0.0, value: 10, description: "Fine planks for interiors. Stations pay well for a touch of home." },
    ItemDef { name: "Redwood beams", category: Category::Component, rarity: Rarity::Rare, stack: 30, food: 0.0, value: 22, description: "Long structural beams from giant conifers. A luxury off-world." },
];

impl ItemId {
    pub fn def(self) -> &'static ItemDef {
        &ITEMS[self.0 as usize]
    }
}

/// Stacks of items. A fixed number of slots, each holding up to its item's stack size.
#[derive(Component, Clone, Debug)]
pub struct Inventory {
    pub slots: Vec<Option<(ItemId, u32)>>,
}

impl Inventory {
    pub fn new(slots: usize) -> Self {
        Self { slots: vec![None; slots] }
    }

    /// Add up to `count`; returns how many fitted.
    pub fn add(&mut self, id: ItemId, count: u32) -> u32 {
        let stack = id.def().stack;
        let mut left = count;
        for slot in self.slots.iter_mut().flatten() {
            if slot.0 == id && slot.1 < stack {
                let n = left.min(stack - slot.1);
                slot.1 += n;
                left -= n;
            }
        }
        for slot in self.slots.iter_mut() {
            if left == 0 {
                break;
            }
            if slot.is_none() {
                let n = left.min(stack);
                *slot = Some((id, n));
                left -= n;
            }
        }
        count - left
    }

    /// Remove one of `id`; false if there is none.
    pub fn take_one(&mut self, id: ItemId) -> bool {
        for slot in self.slots.iter_mut() {
            if let Some((sid, n)) = slot {
                if *sid == id {
                    *n -= 1;
                    if *n == 0 {
                        *slot = None;
                    }
                    return true;
                }
            }
        }
        false
    }

    pub fn count(&self, id: ItemId) -> u32 {
        self.slots.iter().flatten().filter(|s| s.0 == id).map(|s| s.1).sum()
    }

    /// Total base value of everything carried.
    pub fn value(&self) -> u32 {
        self.slots.iter().flatten().map(|(id, n)| id.def().value * n).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_stacks_and_overflows() {
        let mut inv = Inventory::new(2);
        assert_eq!(inv.add(item::BERRIES, 100), 100); // 60 + 40
        assert_eq!(inv.add(item::BERRIES, 30), 20); // fills the second stack, 10 left over
        assert_eq!(inv.count(item::BERRIES), 120);
        assert_eq!(inv.add(item::APPLE, 1), 0); // no free slot
        assert!(inv.take_one(item::BERRIES));
        assert_eq!(inv.count(item::BERRIES), 119);
    }

    #[test]
    fn item_table_matches_ids() {
        assert_eq!(item::AMBER.def().name, "Amber resin");
        assert!(ITEMS.iter().all(|d| d.stack > 0));
    }
}
