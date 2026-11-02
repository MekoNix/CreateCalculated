//! Renders small invented models with textures made right here, never game files.

use createcalc_core::model::Bundle;
use createcalc_icons::assets::{model_path, texture_path, Assets};
use createcalc_icons::generate::{generate, Renderer, Source};
use createcalc_icons::texture::Image;
use std::path::PathBuf;

const RED: [u8; 4] = [200, 40, 40, 255];
const GREEN: [u8; 4] = [40, 200, 40, 255];
const BLUE: [u8; 4] = [40, 40, 200, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const YELLOW: [u8; 4] = [220, 220, 40, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

fn texture(width: u32, height: u32, color: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut image = Image::new(width, height);
    for y in 0..height {
        for x in 0..width {
            image.set(x, y, color(x, y));
        }
    }
    image.encode_png().unwrap()
}

fn solid(color: [u8; 4]) -> Vec<u8> {
    texture(16, 16, move |_, _| color)
}

/// Upper left red, upper right green, lower left blue, lower right yellow.
fn quadrants() -> Vec<u8> {
    texture(16, 16, |x, y| match (x < 8, y < 8) {
        (true, true) => RED,
        (false, true) => GREEN,
        (true, false) => BLUE,
        (false, false) => YELLOW,
    })
}

fn world(models: &[(&str, &str)], textures: Vec<(&str, Vec<u8>)>) -> Assets {
    let mut assets = Assets::default();
    assets.insert(
        model_path("minecraft:block/block"),
        r##"{"gui_light":"side","display":{"gui":{"rotation":[30,225,0],"translation":[0,0,0],"scale":[0.625,0.625,0.625]}}}"##,
    );
    assets.insert(model_path("minecraft:item/generated"), r##"{"parent":"builtin/generated","gui_light":"front"}"##);
    for (location, json) in models {
        assets.insert(model_path(location), *json);
    }
    for (location, png) in textures {
        assets.insert(texture_path(location), png);
    }
    assets
}

fn render(assets: &Assets, id: &str) -> Image {
    Renderer::new(assets, 64).item(id).unwrap_or_else(|e| panic!("{id}: {e}")).image
}

fn near(actual: [u8; 4], expected: [u8; 4], shade: f64) {
    for i in 0..3 {
        let want = expected[i] as f64 * shade;
        assert!((actual[i] as f64 - want).abs() < 3.0, "{actual:?} against {expected:?} shaded by {shade}");
    }
    assert_eq!(actual[3], expected[3]);
}

const FULL_CUBE: &str = r##"{"parent":"block/block","textures":{"d":"test:down","u":"test:up","n":"test:north","s":"test:south","w":"test:west","e":"test:east"},
    "elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{
        "down":{"texture":"#d"},"up":{"texture":"#u"},"north":{"texture":"#n"},
        "south":{"texture":"#s"},"west":{"texture":"#w"},"east":{"texture":"#e"}}}]}"##;

// Where a cube under [30, 225, 0] and 0.625 lands on a 64 by 64 picture.
const ON_TOP: (u32, u32) = (32, 14);
const ON_EAST: (u32, u32) = (16, 38);
const ON_NORTH: (u32, u32) = (48, 38);

#[test]
fn a_block_shows_top_east_and_north_shaded_like_the_game() {
    let assets = world(
        &[("test:item/cube", FULL_CUBE)],
        vec![("test:up", solid(RED)), ("test:east", solid(GREEN)), ("test:north", solid(BLUE)), ("test:down", solid(WHITE)), ("test:south", solid(WHITE)), ("test:west", solid(WHITE))],
    );
    let image = render(&assets, "test:cube");
    near(image.get(ON_TOP.0, ON_TOP.1), RED, 1.0);
    near(image.get(ON_EAST.0, ON_EAST.1), GREEN, 0.65);
    near(image.get(ON_NORTH.0, ON_NORTH.1), BLUE, 0.40);
    assert_eq!(image.get(0, 0), CLEAR);
    assert_eq!(image.get(63, 63), CLEAR);
    assert_eq!(image.get(0, 63), CLEAR);
}

#[test]
fn side_faces_are_not_mirrored() {
    let split = || texture(16, 16, |x, _| if x < 8 { RED } else { BLUE });
    let assets = world(
        &[("test:item/cube", FULL_CUBE)],
        vec![("test:up", solid(WHITE)), ("test:east", split()), ("test:north", split()), ("test:down", solid(WHITE)), ("test:south", solid(WHITE)), ("test:west", solid(WHITE))],
    );
    let image = render(&assets, "test:cube");
    // Each side reads left to right on the screen, as the texture does.
    near(image.get(9, 35), RED, 0.65);
    near(image.get(26, 43), BLUE, 0.65);
    near(image.get(38, 43), RED, 0.40);
    near(image.get(54, 35), BLUE, 0.40);
}

#[test]
fn top_face_uv_runs_along_x_and_z() {
    let model = r##"{"parent":"block/block","textures":{"t":"test:t"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#t"}}}]}"##;
    let assets = world(&[("test:item/top", model)], vec![("test:t", quadrants())]);
    let image = render(&assets, "test:top");
    // u follows x and v follows z, which puts the texture's origin in the right corner.
    near(image.get(46, 15), RED, 1.0);
    near(image.get(32, 22), GREEN, 1.0);
    near(image.get(32, 8), BLUE, 1.0);
    near(image.get(18, 15), YELLOW, 1.0);
}

#[test]
fn face_rotation_turns_the_texture() {
    let model = r##"{"parent":"block/block","textures":{"t":"test:t"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#t","uv":[0,0,16,16],"rotation":90}}}]}"##;
    let assets = world(&[("test:item/top", model)], vec![("test:t", quadrants())]);
    let image = render(&assets, "test:top");
    near(image.get(46, 15), BLUE, 1.0);
    near(image.get(32, 22), RED, 1.0);
    near(image.get(32, 8), YELLOW, 1.0);
    near(image.get(18, 15), GREEN, 1.0);
}

#[test]
fn explicit_uv_picks_part_of_the_texture_without_bleeding() {
    let model = r##"{"parent":"block/block","textures":{"t":"test:t"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#t","uv":[8,0,16,8]}}}]}"##;
    let assets = world(&[("test:item/top", model)], vec![("test:t", quadrants())]);
    let image = render(&assets, "test:top");
    let top: Vec<[u8; 4]> = image.pixels.iter().copied().filter(|p| p[3] != 0).collect();
    assert!(!top.is_empty());
    assert!(top.iter().all(|p| *p == GREEN), "only the green quadrant may show");
}

#[test]
fn back_faces_are_not_drawn() {
    // South and west look away from this view, down too.
    let hidden = r##"{"parent":"block/block","textures":{"t":"test:t"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"south":{"texture":"#t"},"west":{"texture":"#t"},"down":{"texture":"#t"}}}]}"##;
    let assets = world(&[("test:item/hidden", hidden)], vec![("test:t", solid(RED))]);
    assert!(Renderer::new(&assets, 64).item("test:hidden").is_err());
}

#[test]
fn depth_decides_not_the_order_of_elements() {
    let near_cube = r##"{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#a"},"east":{"texture":"#a"},"north":{"texture":"#a"}}}"##;
    let inner = r##"{"from":[6,6,6],"to":[10,10,10],"faces":{"up":{"texture":"#b"},"east":{"texture":"#b"},"north":{"texture":"#b"}}}"##;
    for order in [format!("{near_cube},{inner}"), format!("{inner},{near_cube}")] {
        let model = format!(r##"{{"parent":"block/block","textures":{{"a":"test:a","b":"test:b"}},"elements":[{order}]}}"##);
        let assets = world(&[("test:item/two", &model)], vec![("test:a", solid(RED)), ("test:b", solid(BLUE))]);
        let image = render(&assets, "test:two");
        assert!(image.pixels.iter().all(|p| p[2] < 100), "the inner cube must stay hidden");
    }
}

#[test]
fn transparent_texels_show_what_is_behind_and_faint_ones_are_cut() {
    let glass = r##"{"parent":"block/block","textures":{"a":"test:back","b":"test:front"},"elements":[
        {"from":[2,2,2],"to":[14,14,14],"faces":{"up":{"texture":"#a"}}},
        {"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#b"}}}]}"##;
    let half = [0, 0, 255, 128];
    let front = texture(16, 16, move |x, _| if x < 8 { half } else { [0, 0, 255, 10] });
    let assets = world(&[("test:item/glass", glass)], vec![("test:back", solid(RED)), ("test:front", front)]);
    let image = render(&assets, "test:glass");
    // Half covered: both colours mix. Below the cutoff: only the red one behind.
    let blended = image.get(39, 14);
    assert!(blended[0] > 70 && blended[0] < 130 && blended[2] > 90 && blended[2] < 150, "{blended:?}");
    assert_eq!(blended[3], 255);
    near(image.get(23, 16), RED, 1.0);
}

#[test]
fn translucent_order_does_not_matter() {
    let solid_behind = r##"{"from":[2,2,2],"to":[14,14,14],"faces":{"up":{"texture":"#a"}}}"##;
    let glass_front = r##"{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#b"}}}"##;
    let render_order = |order: String| {
        let model = format!(r##"{{"parent":"block/block","textures":{{"a":"test:a","b":"test:b"}},"elements":[{order}]}}"##);
        let assets = world(&[("test:item/g", &model)], vec![("test:a", solid(RED)), ("test:b", solid([0, 0, 255, 128]))]);
        render(&assets, "test:g")
    };
    let one = render_order(format!("{solid_behind},{glass_front}"));
    let two = render_order(format!("{glass_front},{solid_behind}"));
    assert_eq!(one, two);
}

#[test]
fn generated_items_are_the_flat_texture_scaled_by_whole_pixels() {
    let model = r##"{"parent":"item/generated","textures":{"layer0":"test:flat"}}"##;
    let assets = world(&[("test:item/flat", model)], vec![("test:flat", quadrants())]);
    let image = render(&assets, "test:flat");
    assert_eq!(image.get(0, 0), RED);
    assert_eq!(image.get(31, 31), RED);
    assert_eq!(image.get(32, 0), GREEN);
    assert_eq!(image.get(63, 0), GREEN);
    assert_eq!(image.get(0, 63), BLUE);
    assert_eq!(image.get(63, 63), YELLOW);
}

#[test]
fn layers_stack_and_a_hole_shows_the_layer_below() {
    let model = r##"{"parent":"item/generated","textures":{"layer0":"test:base","layer1":"test:top"}}"##;
    let top = texture(16, 16, |x, _| if x < 8 { CLEAR } else { BLUE });
    let assets = world(&[("test:item/two", model)], vec![("test:base", solid(RED)), ("test:top", top)]);
    let image = render(&assets, "test:two");
    assert_eq!(image.get(4, 30), RED);
    assert_eq!(image.get(60, 30), BLUE);
}

#[test]
fn animated_textures_use_their_first_frame_on_models_too() {
    let strip = texture(16, 48, |_, y| if y < 16 { GREEN } else { RED });
    let mut assets = world(&[("test:item/lamp", r##"{"parent":"item/generated","textures":{"layer0":"test:lamp"}}"##)], vec![("test:lamp", strip)]);
    assets.insert("assets/test/textures/lamp.png.mcmeta", r##"{"animation":{"frametime":2}}"##);
    let image = render(&assets, "test:lamp");
    assert!(image.pixels.iter().all(|p| *p == GREEN));
}

#[test]
fn game_coded_tints_colour_the_right_layer() {
    let model = r##"{"parent":"item/generated","textures":{"layer0":"test:overlay","layer1":"test:bottle"}}"##;
    let bottle = texture(16, 16, |x, _| if x < 8 { CLEAR } else { WHITE });
    let mut assets = world(&[], vec![("test:overlay", solid(WHITE)), ("test:bottle", bottle)]);
    assets.insert(model_path("minecraft:item/potion"), model);
    let image = render(&assets, "minecraft:potion");
    near(image.get(4, 30), [0x38, 0x5D, 0xC6, 255], 1.0);
    assert_eq!(image.get(60, 30), WHITE);
}

#[test]
fn display_translation_moves_the_picture() {
    let model = r##"{"textures":{"t":"test:t"},"gui_light":"front","display":{"gui":{"translation":[4,0,0]}},
        "elements":[{"from":[6,6,8],"to":[10,10,8],"faces":{"south":{"texture":"#t"}}}]}"##;
    let assets = world(&[("test:item/moved", model)], vec![("test:t", solid(RED))]);
    let image = render(&assets, "test:moved");
    assert_eq!(image.get(32, 32), CLEAR);
    assert_eq!(image.get(48, 32), RED);
}

#[test]
fn a_model_larger_than_its_slot_is_scaled_to_fit() {
    let model = r##"{"textures":{"t":"test:t"},"gui_light":"front",
        "elements":[{"from":[-16,-16,8],"to":[32,32,8],"faces":{"south":{"texture":"#t"}}}]}"##;
    let assets = world(&[("test:item/big", model)], vec![("test:t", quadrants())]);
    let image = render(&assets, "test:big");
    assert_eq!(image.get(0, 0), RED);
    assert_eq!(image.get(63, 63), YELLOW);
}

#[test]
fn obj_models_go_through_the_same_renderer() {
    let model = r##"{"parent":"block/block","loader":"neoforge:obj","flip_v":true,"model":"test:models/o.obj","textures":{"skin":"test:t"},"gui_light":"front","display":{"gui":{"rotation":[0,0,0],"scale":[1,1,1]}}}"##;
    let mut assets = world(&[("test:item/o", model)], vec![("test:t", quadrants())]);
    assets.insert("assets/test/models/o.mtl", "newmtl m\nmap_Kd #skin\n");
    assets.insert(
        "assets/test/models/o.obj",
        "mtllib o.mtl\nusemtl m\nv 0 0 0.5\nv 1 0 0.5\nv 1 1 0.5\nv 0 1 0.5\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvn 0 0 1\nf 1/1/1 2/2/1 3/3/1 4/4/1\n",
    );
    let image = render(&assets, "test:o");
    assert_eq!(image.get(0, 0), RED);
    assert_eq!(image.get(63, 0), GREEN);
    assert_eq!(image.get(0, 63), BLUE);
}

#[test]
fn composite_models_draw_every_part() {
    let model = r##"{"loader":"neoforge:composite","parent":"block/block","textures":{"a":"test:a"},"children":{
        "base":{"elements":[{"from":[0,0,0],"to":[16,8,16],"faces":{"east":{"texture":"#a"},"north":{"texture":"#a"}}}]},
        "lid":{"textures":{"b":"test:b"},"elements":[{"from":[0,8,0],"to":[16,16,16],"faces":{"up":{"texture":"#b"}}}]}}}"##;
    let assets = world(&[("test:item/c", model)], vec![("test:a", solid(RED)), ("test:b", solid(BLUE))]);
    let image = render(&assets, "test:c");
    near(image.get(ON_TOP.0, ON_TOP.1), BLUE, 1.0);
    near(image.get(16, 45), RED, 0.65);
}

#[test]
fn a_face_with_a_lost_texture_is_left_out_and_reported() {
    let model = r##"{"parent":"block/block","textures":{"a":"test:a","b":"test:gone"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#a"},"east":{"texture":"#b"},"north":{"texture":"#nothing"}}}]}"##;
    let assets = world(&[("test:item/lost", model)], vec![("test:a", solid(RED))]);
    let rendered = Renderer::new(&assets, 64).item("test:lost").unwrap();
    assert_eq!(rendered.source, Source::Model);
    assert_eq!(rendered.missing.len(), 2);
    near(rendered.image.get(ON_TOP.0, ON_TOP.1), RED, 1.0);
    assert_eq!(rendered.image.get(ON_EAST.0, ON_EAST.1), CLEAR);
}

#[test]
fn items_from_game_code_use_their_box_model_or_stay_without_a_picture() {
    let entity = r##"{"parent":"builtin/entity","textures":{"particle":"test:wood"},"display":{"gui":{"rotation":[30,45,0],"scale":[0.625,0.625,0.625]}}}"##;
    let mut assets = world(&[], vec![("minecraft:entity/chest/normal", texture(64, 64, |_, _| RED)), ("test:wood", solid(GREEN))]);
    assets.insert(model_path("minecraft:item/chest"), entity);
    assets.insert(model_path("minecraft:item/dragon_head"), entity);
    let chest = Renderer::new(&assets, 64).item("minecraft:chest").unwrap();
    assert_eq!(chest.source, Source::GameCode);
    assert!(chest.image.pixels.iter().any(|p| p[3] == 255));
    assert!(Renderer::new(&assets, 64).item("minecraft:dragon_head").is_err());
}

#[test]
fn an_item_without_a_model_falls_back_to_a_texture_of_its_name() {
    let assets = world(&[], vec![("test:item/plain", solid(RED)), ("test:block/brick", solid(GREEN))]);
    let plain = Renderer::new(&assets, 64).item("test:plain").unwrap();
    assert_eq!(plain.source, Source::Texture);
    assert_eq!(plain.image.get(32, 32), RED);
    assert_eq!(Renderer::new(&assets, 64).item("test:brick").unwrap().image.get(32, 32), GREEN);
    assert!(Renderer::new(&assets, 64).item("test:nothing").is_err());
}

#[test]
fn fluids_are_the_first_frame_of_their_still_texture_and_can_be_tinted() {
    let grey = [200, 200, 200, 255];
    let strip = texture(16, 32, move |_, y| if y < 16 { grey } else { RED });
    let mut assets = world(&[], vec![("minecraft:block/water_still", strip), ("test:fluid/oil_still", solid(GREEN)), ("create:fluid/potion_still", solid(WHITE))]);
    assets.insert("assets/minecraft/textures/block/water_still.png.mcmeta", r##"{"animation":{}}"##);
    let mut renderer = Renderer::new(&assets, 32);
    let water = renderer.fluid("fluid:minecraft:water").unwrap();
    near(water.get(16, 16), [0x3F, 0x76, 0xE4, 255], 200.0 / 255.0);
    assert_eq!(renderer.fluid("fluid:test:oil").unwrap().get(0, 0), GREEN);
    near(renderer.fluid("fluid:create:potion[minecraft:strength]").unwrap().get(0, 0), [0xFF, 0xC7, 0x00, 255], 1.0);
    // A fluid of the same name under another namespace stands in.
    assert_eq!(renderer.fluid("fluid:other:oil").unwrap().get(0, 0), GREEN);
    assert!(renderer.fluid("fluid:test:tar").is_err());
}

struct TempFolder(PathBuf);

impl TempFolder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("createcalc-icons-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn generate_writes_pictures_an_index_and_tag_aliases() {
    let folder = TempFolder::new("generate");
    let assets = world(
        &[
            ("test:item/gear", r##"{"parent":"item/generated","textures":{"layer0":"test:gear"}}"##),
            ("test:item/empty", r##"{"parent":"block/block"}"##),
            ("test:item/gear_gold_trim", r##"{"parent":"item/generated","textures":{"layer0":"test:gear"}}"##),
        ],
        vec![("test:gear", solid(RED)), ("test:fluid/oil_still", solid(GREEN))],
    );
    let bundle: Bundle = serde_json::from_str(
        r##"{"recipes":[{"id":"test:a","kind":"mixing","inputs":[{"res":"#test:gears","amount":1},{"res":"#test:unknown","amount":1},{"res":"#fluid:test:oils","amount":100}],
            "outputs":[{"res":"test:gear","amount":1,"chance":1},{"res":"test:ghost","amount":1,"chance":1},{"res":"fluid:test:oil","amount":1,"chance":1}]}],
            "tags":{"#test:gears":["test:missing","test:gear"],"#fluid:test:oils":["fluid:test:oil"],"#test:unknown":["test:nothing"]},"names":{}}"##,
    )
    .unwrap();
    let report = generate(&assets, Some(&bundle), 32, &folder.0).unwrap();

    let index: serde_json::Value = serde_json::from_slice(&std::fs::read(folder.0.join("index.json")).unwrap()).unwrap();
    assert_eq!(index["size"], 32);
    assert!(!index["version"].as_str().unwrap().is_empty());
    let icons = &index["icons"];
    assert_eq!(icons["test:gear"], "items/test/gear.png");
    assert_eq!(icons["fluid:test:oil"], "fluids/test/oil.png");
    assert_eq!(icons["#test:gears"], "items/test/gear.png", "a tag shows its first member that has a picture");
    assert_eq!(icons["#fluid:test:oils"], "fluids/test/oil.png");
    assert!(icons.get("#test:unknown").is_none());
    assert!(icons.get("test:empty").is_none());
    assert!(icons.get("test:ghost").is_none());
    assert!(icons.get("test:gear_gold_trim").is_none(), "trim variants are not items");

    let png = std::fs::read(folder.0.join("items/test/gear.png")).unwrap();
    let image = Image::decode_png(&png).unwrap();
    assert_eq!((image.width, image.height), (32, 32));
    assert_eq!(image.get(0, 0), RED);

    let coverage = report.coverage.unwrap();
    assert_eq!(coverage.items, (1, 2));
    assert_eq!(coverage.fluids, (1, 1));
    assert_eq!(coverage.tags, (2, 3));
    assert!(coverage.missing.contains(&"test:ghost".to_string()));
    assert!(report.failed.iter().any(|(id, _)| id == "test:empty"));
}

#[test]
fn ids_from_a_bundle_cannot_write_outside_the_folder() {
    let folder = TempFolder::new("escape");
    let assets = world(&[], vec![("test:item/x", solid(RED))]);
    let bundle: Bundle = serde_json::from_str(
        r##"{"recipes":[{"id":"t:a","kind":"mixing","inputs":[],"outputs":[{"res":"test:../../escaped","amount":1,"chance":1}]}],"tags":{},"names":{}}"##,
    )
    .unwrap();
    let report = generate(&assets, Some(&bundle), 16, &folder.0.join("inner")).unwrap();
    assert_eq!(report.written, 0);
    assert!(!folder.0.join("escaped.png").exists());
}
