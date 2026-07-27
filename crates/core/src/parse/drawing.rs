//! DrawingML: colours, fills, lines, effects, transforms and geometry.
//!
//! These elements appear identically in slides, layouts, masters, themes, tables and
//! charts, so every part parser routes through here.

use quick_xml::events::{BytesStart, Event};

use crate::dl::Color;
use crate::emu::Emu;
use crate::model::color::{preset_color, ColorMod, ColorRef, ColorSpec, SchemeColor};
use crate::model::fill::{
    ArrowEnd, ArrowType, BlipFill, BlipMode, DashStyle, Effects, Fill, Glow, GradientFill,
    GradientKind, GradientStopSpec, Line, LineCapStyle, LineJoinStyle, OuterShadow, PatternFill,
};
use crate::model::geometry::{
    CustomGeometry, Expr, GeomCommand, GeomPath, Geometry, GroupTransform, Guide, PathFillMode,
    Transform2D,
};

use super::xml::{
    attr, attr_bool, attr_f32, attr_i32, attr_i64, attr_percent, attr_u32, is, local_name,
    skip_element, Reader,
};

/// Iterates the direct children of the element the reader has just entered.
///
/// The callback returns whether it consumed the child's subtree. Anything it did not
/// consume is skipped, so an element this build does not understand cannot leak its own
/// children into the parent's child stream and be mistaken for siblings.
pub fn children<'a, F>(r: &mut Reader<'a>, parent: &[u8], mut f: F)
where
    F: FnMut(&mut Reader<'a>, &BytesStart<'_>, bool) -> bool,
{
    let mut buf = Vec::new();
    loop {
        let (elem, empty) = {
            match r.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => (e.to_owned(), false),
                Ok(Event::Empty(e)) => (e.to_owned(), true),
                Ok(Event::End(e)) => {
                    if local_name(e.name().as_ref()) == parent {
                        break;
                    }
                    buf.clear();
                    continue;
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    log::debug!(
                        "xml error inside <{}>: {e}",
                        String::from_utf8_lossy(parent)
                    );
                    break;
                }
                _ => {
                    buf.clear();
                    continue;
                }
            }
        };
        let name = local_name(elem.name().as_ref()).to_vec();
        let consumed = f(r, &elem, empty);
        if !consumed && !empty {
            skip_element(r, &name);
        }
        buf.clear();
    }
}

// ---------------------------------------------------------------- colours

/// Parses a colour element (`<a:srgbClr>` and friends) including its modifier children.
pub fn parse_color_element(
    r: &mut Reader<'_>,
    e: &BytesStart<'_>,
    empty: bool,
) -> Option<ColorRef> {
    let name = local_name(e.name().as_ref()).to_vec();
    let spec = match name.as_slice() {
        b"srgbClr" => ColorSpec::Srgb(
            attr(e, b"val")
                .as_deref()
                .and_then(Color::from_hex)
                .unwrap_or(Color::BLACK),
        ),
        b"schemeClr" => {
            let val = attr(e, b"val").unwrap_or_default();
            if val == "phClr" {
                ColorSpec::Placeholder
            } else {
                match SchemeColor::parse(&val) {
                    Some(s) => ColorSpec::Scheme(s),
                    None => {
                        log::debug!("unknown scheme colour {val:?}");
                        ColorSpec::Srgb(Color::BLACK)
                    }
                }
            }
        }
        b"sysClr" => ColorSpec::System(
            attr(e, b"lastClr")
                .as_deref()
                .and_then(Color::from_hex)
                // No system palette in a browser; `windowText` is the common case.
                .unwrap_or(Color::BLACK),
        ),
        b"prstClr" => ColorSpec::Preset(
            attr(e, b"val")
                .as_deref()
                .and_then(preset_color)
                .unwrap_or(Color::BLACK),
        ),
        b"hslClr" => ColorSpec::Hsl {
            h: attr_f32(e, b"hue").unwrap_or(0.0) / 60_000.0,
            s: attr_percent(e, b"sat").unwrap_or(0.0),
            l: attr_percent(e, b"lum").unwrap_or(0.0),
        },
        b"scrgbClr" => {
            // Linear-space percentages. Converted to sRGB so the rest of the pipeline
            // only ever deals in one colour space.
            let to_srgb = |v: f32| {
                let v = v.clamp(0.0, 1.0);
                let s = if v <= 0.003_130_8 {
                    v * 12.92
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                };
                (s * 255.0).round().clamp(0.0, 255.0) as u8
            };
            ColorSpec::Srgb(Color::rgb(
                to_srgb(attr_percent(e, b"r").unwrap_or(0.0)),
                to_srgb(attr_percent(e, b"g").unwrap_or(0.0)),
                to_srgb(attr_percent(e, b"b").unwrap_or(0.0)),
            ))
        }
        _ => return None,
    };

    let mut color = ColorRef::new(spec);
    if !empty {
        children(r, &name, |_r, child, _child_empty| {
            if let Some(m) = parse_color_mod(child) {
                color.mods.push(m);
            }
            false
        });
    }
    Some(color)
}

fn parse_color_mod(e: &BytesStart<'_>) -> Option<ColorMod> {
    let pct = || attr_percent(e, b"val").unwrap_or(0.0);
    Some(match local_name(e.name().as_ref()) {
        b"alpha" => ColorMod::Alpha(pct()),
        b"alphaMod" => ColorMod::AlphaMod(pct()),
        b"alphaOff" => ColorMod::AlphaOff(pct()),
        b"tint" => ColorMod::Tint(pct()),
        b"shade" => ColorMod::Shade(pct()),
        b"lumMod" => ColorMod::LumMod(pct()),
        b"lumOff" => ColorMod::LumOff(pct()),
        b"satMod" => ColorMod::SatMod(pct()),
        b"satOff" => ColorMod::SatOff(pct()),
        b"hueMod" => ColorMod::HueMod(pct()),
        // Hue offsets are an angle, not a percentage.
        b"hueOff" => ColorMod::HueOff(attr_f32(e, b"val").unwrap_or(0.0) / 60_000.0),
        b"gray" => ColorMod::Gray,
        b"inv" => ColorMod::Inverse,
        _ => return None,
    })
}

/// Parses a container that holds exactly one colour element, e.g. `<a:fgClr>`,
/// `<a:lnRef>`, `<a:clrFrom>`.
pub fn parse_color_container(r: &mut Reader<'_>, container: &[u8]) -> Option<ColorRef> {
    let mut out = None;
    children(r, container, |r, e, empty| {
        if out.is_none() {
            if let Some(c) = parse_color_element(r, e, empty) {
                out = Some(c);
                return !empty;
            }
        }
        false
    });
    out
}

// ---------------------------------------------------------------- fills

/// Recognises the fill elements. Returns `None` for anything that is not a fill, so
/// callers can use it inside a generic child loop.
pub fn parse_fill(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> Option<Fill> {
    let qname = e.name();
    match local_name(qname.as_ref()) {
        b"noFill" => Some(Fill::NoFill),
        b"grpFill" => Some(Fill::Group),
        b"solidFill" => {
            if empty {
                return Some(Fill::NoFill);
            }
            Some(match parse_color_container(r, b"solidFill") {
                Some(c) => Fill::Solid(c),
                None => Fill::NoFill,
            })
        }
        b"gradFill" => Some(Fill::Gradient(parse_gradient(r, e, empty))),
        b"blipFill" => Some(Fill::Blip(parse_blip_fill(r, e, empty))),
        b"pattFill" => Some(Fill::Pattern(parse_pattern_fill(r, e, empty))),
        _ => None,
    }
}

fn parse_gradient(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> GradientFill {
    let mut g = GradientFill {
        kind: GradientKind::Linear,
        stops: Vec::new(),
        angle_deg: 0.0,
        scaled: attr_bool(e, b"scaled").unwrap_or(false),
        focus: None,
    };
    if empty {
        return g;
    }
    children(r, b"gradFill", |r, child, child_empty| {
        match local_name(child.name().as_ref()) {
            b"gsLst" => {
                if child_empty {
                    return true;
                }
                children(r, b"gsLst", |r, gs, gs_empty| {
                    if !is(gs, b"gs") {
                        return false;
                    }
                    let pos = attr_percent(gs, b"pos").unwrap_or(0.0);
                    let color = if gs_empty {
                        None
                    } else {
                        parse_color_container(r, b"gs")
                    };
                    if let Some(color) = color {
                        g.stops.push(GradientStopSpec { pos, color });
                    }
                    !gs_empty
                });
                true
            }
            b"lin" => {
                // `ang` is in 60000ths of a degree, clockwise from +x.
                g.angle_deg = attr_i32(child, b"ang").unwrap_or(0) as f32 / 60_000.0;
                g.scaled = attr_bool(child, b"scaled").unwrap_or(g.scaled);
                g.kind = GradientKind::Linear;
                false
            }
            b"path" => {
                g.kind = match attr(child, b"path").as_deref() {
                    Some("circle") => GradientKind::Radial,
                    Some("rect") => GradientKind::Rect,
                    Some("shape") => GradientKind::Shape,
                    _ => GradientKind::Path,
                };
                if child_empty {
                    return true;
                }
                children(r, b"path", |_r, fr, _e| {
                    if is(fr, b"fillToRect") {
                        g.focus = Some([
                            attr_percent(fr, b"l").unwrap_or(0.0),
                            attr_percent(fr, b"t").unwrap_or(0.0),
                            attr_percent(fr, b"r").unwrap_or(0.0),
                            attr_percent(fr, b"b").unwrap_or(0.0),
                        ]);
                    }
                    false
                });
                true
            }
            _ => false,
        }
    });
    g.stops.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    g
}

fn parse_blip_fill(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> BlipFill {
    let mut f = BlipFill::default();
    // `<a:blipFill rotWithShape="1">` is the default for pictures.
    let _ = attr_bool(e, b"rotWithShape");
    if empty {
        return f;
    }
    children(r, b"blipFill", |r, child, child_empty| {
        match local_name(child.name().as_ref()) {
            b"blip" => {
                f.embed_id = super::xml::r_attr(child, b"embed");
                if child_empty {
                    return true;
                }
                children(r, b"blip", |_r, mod_e, _me| {
                    if is(mod_e, b"alphaModFix") {
                        f.alpha = attr_percent(mod_e, b"amt").unwrap_or(1.0);
                    }
                    false
                });
                true
            }
            b"srcRect" => {
                f.src_rect = parse_rect_insets(child);
                false
            }
            b"stretch" => {
                f.mode = BlipMode::Stretch;
                if child_empty {
                    return true;
                }
                children(r, b"stretch", |_r, fr, _e| {
                    if is(fr, b"fillRect") {
                        f.fill_rect = parse_rect_insets(fr);
                    }
                    false
                });
                true
            }
            b"tile" => {
                f.mode = BlipMode::Tile;
                false
            }
            _ => false,
        }
    });
    f
}

/// `<a:srcRect l=".." t=".." r=".." b="..">` — insets as 0..1 fractions.
fn parse_rect_insets(e: &BytesStart<'_>) -> [f32; 4] {
    [
        attr_percent(e, b"l").unwrap_or(0.0),
        attr_percent(e, b"t").unwrap_or(0.0),
        attr_percent(e, b"r").unwrap_or(0.0),
        attr_percent(e, b"b").unwrap_or(0.0),
    ]
}

fn parse_pattern_fill(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> PatternFill {
    let mut p = PatternFill {
        foreground: ColorRef::srgb(Color::BLACK),
        background: ColorRef::srgb(Color::WHITE),
        preset: attr(e, b"prst").unwrap_or_else(|| "pct50".into()),
    };
    if empty {
        return p;
    }
    children(r, b"pattFill", |r, child, child_empty| {
        match local_name(child.name().as_ref()) {
            b"fgClr" if !child_empty => {
                if let Some(c) = parse_color_container(r, b"fgClr") {
                    p.foreground = c;
                }
                true
            }
            b"bgClr" if !child_empty => {
                if let Some(c) = parse_color_container(r, b"bgClr") {
                    p.background = c;
                }
                true
            }
            _ => false,
        }
    });
    p
}

// ---------------------------------------------------------------- lines

/// Parses an `<a:ln>`-shaped element.
///
/// The container name is taken from the element rather than assumed to be `ln`, because
/// table cell borders use the identical content model under `lnL`/`lnR`/`lnT`/`lnB`.
/// Hard-coding `ln` here makes a border parse run off the end of its parent and swallow
/// every following sibling.
pub fn parse_line(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> Line {
    let container = local_name(e.name().as_ref()).to_vec();
    let mut line = Line {
        width: attr_i64(e, b"w"),
        cap: attr(e, b"cap").as_deref().and_then(|c| match c {
            "flat" => Some(LineCapStyle::Flat),
            "rnd" => Some(LineCapStyle::Round),
            "sq" => Some(LineCapStyle::Square),
            _ => None,
        }),
        ..Default::default()
    };
    if empty {
        return line;
    }
    children(r, &container, |r, child, child_empty| {
        let name = local_name(child.name().as_ref()).to_vec();
        if let Some(f) = parse_fill(r, child, child_empty) {
            line.fill = f;
            return !child_empty;
        }
        match name.as_slice() {
            b"prstDash" => {
                line.dash = attr(child, b"val").as_deref().and_then(DashStyle::parse);
                false
            }
            b"custDash" => {
                // Custom dash arrays are rare; approximate with a plain dash so the line
                // at least reads as non-solid.
                line.dash = Some(DashStyle::Dash);
                false
            }
            b"round" => {
                line.join = Some(LineJoinStyle::Round);
                false
            }
            b"bevel" => {
                line.join = Some(LineJoinStyle::Bevel);
                false
            }
            b"miter" => {
                line.join = Some(LineJoinStyle::Miter);
                false
            }
            b"headEnd" => {
                line.head = Some(parse_arrow(child));
                false
            }
            b"tailEnd" => {
                line.tail = Some(parse_arrow(child));
                false
            }
            _ => false,
        }
    });
    line
}

fn parse_arrow(e: &BytesStart<'_>) -> ArrowEnd {
    let size = |v: Option<String>| match v.as_deref() {
        Some("sm") => 2.0,
        Some("lg") => 4.0,
        _ => 3.0,
    };
    ArrowEnd {
        kind: attr(e, b"type")
            .as_deref()
            .map(ArrowType::parse)
            .unwrap_or(ArrowType::None),
        width: size(attr(e, b"w")),
        length: size(attr(e, b"len")),
    }
}

// ---------------------------------------------------------------- effects

pub fn parse_effects(r: &mut Reader<'_>, empty: bool) -> Effects {
    let mut fx = Effects::default();
    if empty {
        return fx;
    }
    children(r, b"effectLst", |r, child, child_empty| {
        match local_name(child.name().as_ref()) {
            b"outerShdw" => {
                let mut shadow = OuterShadow {
                    blur: attr_i64(child, b"blurRad").unwrap_or(0),
                    distance: attr_i64(child, b"dist").unwrap_or(0),
                    direction_deg: attr_i32(child, b"dir").unwrap_or(0) as f32 / 60_000.0,
                    color: ColorRef::srgb(Color::rgba(0, 0, 0, 128)),
                };
                if !child_empty {
                    if let Some(c) = parse_color_container(r, b"outerShdw") {
                        shadow.color = c;
                    }
                }
                fx.outer_shadow = Some(shadow);
                !child_empty
            }
            b"glow" => {
                let mut glow = Glow {
                    radius: attr_i64(child, b"rad").unwrap_or(0),
                    color: ColorRef::srgb(Color::WHITE),
                };
                if !child_empty {
                    if let Some(c) = parse_color_container(r, b"glow") {
                        glow.color = c;
                    }
                }
                fx.glow = Some(glow);
                !child_empty
            }
            b"softEdge" => {
                fx.soft_edge = attr_i64(child, b"rad");
                false
            }
            _ => false,
        }
    });
    fx
}

// ---------------------------------------------------------------- transforms

/// Parses `<a:xfrm>`. `specified` is set so callers can tell an absent transform (which
/// inherits from a placeholder) from one that is present and zero.
pub fn parse_xfrm(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> Transform2D {
    let mut t = Transform2D {
        rotation: attr_i32(e, b"rot").unwrap_or(0),
        flip_h: attr_bool(e, b"flipH").unwrap_or(false),
        flip_v: attr_bool(e, b"flipV").unwrap_or(false),
        specified: false,
        ..Default::default()
    };
    if empty {
        return t;
    }
    children(r, b"xfrm", |_r, child, _ce| {
        match local_name(child.name().as_ref()) {
            b"off" => {
                t.offset_x = attr_i64(child, b"x").unwrap_or(0);
                t.offset_y = attr_i64(child, b"y").unwrap_or(0);
                t.specified = true;
            }
            b"ext" => {
                t.extent_x = attr_i64(child, b"cx").unwrap_or(0);
                t.extent_y = attr_i64(child, b"cy").unwrap_or(0);
                t.specified = true;
            }
            _ => {}
        }
        false
    });
    t
}

/// Parses a group's `<a:xfrm>`, which additionally carries the child coordinate space.
pub fn parse_group_xfrm(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> GroupTransform {
    let mut g = GroupTransform {
        xfrm: Transform2D {
            rotation: attr_i32(e, b"rot").unwrap_or(0),
            flip_h: attr_bool(e, b"flipH").unwrap_or(false),
            flip_v: attr_bool(e, b"flipV").unwrap_or(false),
            ..Default::default()
        },
        ..Default::default()
    };
    if empty {
        return g;
    }
    children(r, b"xfrm", |_r, child, _ce| {
        match local_name(child.name().as_ref()) {
            b"off" => {
                g.xfrm.offset_x = attr_i64(child, b"x").unwrap_or(0);
                g.xfrm.offset_y = attr_i64(child, b"y").unwrap_or(0);
                g.xfrm.specified = true;
            }
            b"ext" => {
                g.xfrm.extent_x = attr_i64(child, b"cx").unwrap_or(0);
                g.xfrm.extent_y = attr_i64(child, b"cy").unwrap_or(0);
                g.xfrm.specified = true;
            }
            b"chOff" => {
                g.child_offset_x = attr_i64(child, b"x").unwrap_or(0);
                g.child_offset_y = attr_i64(child, b"y").unwrap_or(0);
            }
            b"chExt" => {
                g.child_extent_x = attr_i64(child, b"cx").unwrap_or(0);
                g.child_extent_y = attr_i64(child, b"cy").unwrap_or(0);
            }
            _ => {}
        }
        false
    });
    // A group with no child extent maps 1:1; without this a malformed group collapses
    // every child onto the origin.
    if g.child_extent_x == 0 {
        g.child_extent_x = g.xfrm.extent_x;
    }
    if g.child_extent_y == 0 {
        g.child_extent_y = g.xfrm.extent_y;
    }
    g
}

// ---------------------------------------------------------------- geometry

pub fn parse_preset_geometry(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> Geometry {
    let preset = attr(e, b"prst").unwrap_or_else(|| "rect".into());
    let mut adjustments = Vec::new();
    if !empty {
        children(r, b"prstGeom", |r, child, child_empty| {
            if !is(child, b"avLst") || child_empty {
                return false;
            }
            children(r, b"avLst", |_r, gd, _ge| {
                if is(gd, b"gd") {
                    if let (Some(name), Some(f)) = (attr(gd, b"name"), attr(gd, b"fmla")) {
                        // Adjustment formulas are always `val <n>`.
                        if let Some(v) = f.strip_prefix("val ").and_then(|s| s.trim().parse().ok())
                        {
                            adjustments.push((name, v));
                        }
                    }
                }
                false
            });
            true
        });
    }
    Geometry::Preset {
        preset,
        adjustments,
    }
}

pub fn parse_custom_geometry(r: &mut Reader<'_>, empty: bool) -> Geometry {
    let mut geom = CustomGeometry::default();
    if empty {
        return Geometry::Custom(Box::new(geom));
    }
    children(r, b"custGeom", |r, child, child_empty| {
        match local_name(child.name().as_ref()) {
            b"avLst" if !child_empty => {
                geom.adjust = parse_guide_list(r, b"avLst");
                true
            }
            b"gdLst" if !child_empty => {
                geom.guides = parse_guide_list(r, b"gdLst");
                true
            }
            b"pathLst" if !child_empty => {
                children(r, b"pathLst", |r, p, p_empty| {
                    if !is(p, b"path") {
                        return false;
                    }
                    geom.paths.push(parse_geom_path(r, p, p_empty));
                    !p_empty
                });
                true
            }
            b"rect" => {
                geom.text_rect = Some([
                    Expr::parse(&attr(child, b"l").unwrap_or_default()),
                    Expr::parse(&attr(child, b"t").unwrap_or_default()),
                    Expr::parse(&attr(child, b"r").unwrap_or_default()),
                    Expr::parse(&attr(child, b"b").unwrap_or_default()),
                ]);
                false
            }
            _ => false,
        }
    });
    Geometry::Custom(Box::new(geom))
}

fn parse_guide_list(r: &mut Reader<'_>, container: &[u8]) -> Vec<Guide> {
    let mut guides = Vec::new();
    children(r, container, |_r, gd, _e| {
        if is(gd, b"gd") {
            if let (Some(name), Some(formula)) = (attr(gd, b"name"), attr(gd, b"fmla")) {
                guides.push(Guide { name, formula });
            }
        }
        false
    });
    guides
}

fn parse_geom_path(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> GeomPath {
    let mut path = GeomPath {
        width: attr_i64(e, b"w"),
        height: attr_i64(e, b"h"),
        fill_mode: match attr(e, b"fill").as_deref() {
            Some("none") => PathFillMode::None,
            Some("lighten") => PathFillMode::Lighten,
            Some("lightenLess") => PathFillMode::LightenLess,
            Some("darken") => PathFillMode::Darken,
            Some("darkenLess") => PathFillMode::DarkenLess,
            _ => PathFillMode::Normal,
        },
        stroke: attr_bool(e, b"stroke").unwrap_or(true),
        commands: Vec::new(),
    };
    if empty {
        return path;
    }
    children(r, b"path", |r, cmd, cmd_empty| {
        let name = local_name(cmd.name().as_ref()).to_vec();
        match name.as_slice() {
            b"moveTo" | b"lnTo" => {
                if cmd_empty {
                    return true;
                }
                let pts = parse_points(r, &name);
                if let Some(p) = pts.first() {
                    path.commands.push(if name == b"moveTo" {
                        GeomCommand::MoveTo(p.0.clone(), p.1.clone())
                    } else {
                        GeomCommand::LineTo(p.0.clone(), p.1.clone())
                    });
                }
                true
            }
            b"cubicBezTo" => {
                if cmd_empty {
                    return true;
                }
                let pts = parse_points(r, &name);
                if let (Some(a), Some(b), Some(c)) = (pts.first(), pts.get(1), pts.get(2)) {
                    path.commands.push(GeomCommand::CubicTo(
                        a.0.clone(),
                        a.1.clone(),
                        b.0.clone(),
                        b.1.clone(),
                        c.0.clone(),
                        c.1.clone(),
                    ));
                }
                true
            }
            b"quadBezTo" => {
                if cmd_empty {
                    return true;
                }
                let pts = parse_points(r, &name);
                if let (Some(a), Some(b)) = (pts.first(), pts.get(1)) {
                    path.commands.push(GeomCommand::QuadTo(
                        a.0.clone(),
                        a.1.clone(),
                        b.0.clone(),
                        b.1.clone(),
                    ));
                }
                true
            }
            b"arcTo" => {
                path.commands.push(GeomCommand::ArcTo {
                    wr: Expr::parse(&attr(cmd, b"wR").unwrap_or_default()),
                    hr: Expr::parse(&attr(cmd, b"hR").unwrap_or_default()),
                    start_angle: Expr::parse(&attr(cmd, b"stAng").unwrap_or_default()),
                    swing_angle: Expr::parse(&attr(cmd, b"swAng").unwrap_or_default()),
                });
                false
            }
            b"close" => {
                path.commands.push(GeomCommand::Close);
                false
            }
            _ => false,
        }
    });
    path
}

/// Reads the `<a:pt x=".." y="..">` children of a path command.
fn parse_points(r: &mut Reader<'_>, container: &[u8]) -> Vec<(Expr, Expr)> {
    let mut pts = Vec::new();
    children(r, container, |_r, p, _e| {
        if is(p, b"pt") {
            pts.push((
                Expr::parse(&attr(p, b"x").unwrap_or_default()),
                Expr::parse(&attr(p, b"y").unwrap_or_default()),
            ));
        }
        false
    });
    pts
}

/// Parses whichever of `<a:prstGeom>` / `<a:custGeom>` the element is, or `None`.
pub fn parse_geometry(r: &mut Reader<'_>, e: &BytesStart<'_>, empty: bool) -> Option<Geometry> {
    match local_name(e.name().as_ref()) {
        b"prstGeom" => Some(parse_preset_geometry(r, e, empty)),
        b"custGeom" => Some(parse_custom_geometry(r, empty)),
        _ => None,
    }
}

/// `<a:ext cx cy>` outside an `<a:xfrm>`, used by graphic frames.
pub fn parse_extent(e: &BytesStart<'_>) -> (Emu, Emu) {
    (
        attr_i64(e, b"cx").unwrap_or(0),
        attr_i64(e, b"cy").unwrap_or(0),
    )
}

/// `<p:style>` — the fill/line/effect/font references into the theme's format scheme.
pub fn parse_style_ref(r: &mut Reader<'_>) -> crate::model::shape::StyleRef {
    let mut style = crate::model::shape::StyleRef::default();
    children(r, b"style", |r, child, child_empty| {
        let name = local_name(child.name().as_ref()).to_vec();
        let idx = attr_u32(child, b"idx");
        let color = if child_empty {
            None
        } else {
            parse_color_container(r, &name)
        };
        match name.as_slice() {
            b"fillRef" => {
                style.fill_idx = idx;
                style.fill_color = color;
            }
            b"lnRef" => {
                style.line_idx = idx;
                style.line_color = color;
            }
            b"effectRef" => {
                style.effect_idx = idx;
                style.effect_color = color;
            }
            b"fontRef" => {
                style.font_kind = attr(child, b"idx");
                style.font_color = color;
            }
            _ => return false,
        }
        !child_empty
    });
    style
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::color::SchemeLookup;

    /// Enters the first element of `xml` and hands the reader to `f`.
    fn parse_first<T>(xml: &str, f: impl FnOnce(&mut Reader<'_>, &BytesStart<'_>, bool) -> T) -> T {
        let mut r = Reader::new(xml.as_bytes());
        let mut buf = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let owned = e.to_owned();
                    return f(&mut r, &owned, false);
                }
                Ok(Event::Empty(e)) => {
                    let owned = e.to_owned();
                    return f(&mut r, &owned, true);
                }
                Ok(Event::Eof) => panic!("no element in {xml}"),
                _ => {}
            }
        }
    }

    struct Office;
    impl SchemeLookup for Office {
        fn scheme_color(&self, slot: SchemeColor) -> Color {
            crate::model::theme::ColorScheme::default().get(slot)
        }
    }

    #[test]
    fn srgb_colour_with_a_modifier_stack() {
        let c = parse_first(
            r#"<a:srgbClr val="4472C4"><a:lumMod val="60000"/><a:lumOff val="40000"/></a:srgbClr>"#,
            parse_color_element,
        )
        .expect("colour");
        assert_eq!(c.spec, ColorSpec::Srgb(Color::rgb(0x44, 0x72, 0xC4)));
        assert_eq!(c.mods, vec![ColorMod::LumMod(0.6), ColorMod::LumOff(0.4)]);
    }

    #[test]
    fn scheme_colour_and_phclr_are_distinguished() {
        let a = parse_first(r#"<a:schemeClr val="accent2"/>"#, parse_color_element).expect("a");
        assert_eq!(a.spec, ColorSpec::Scheme(SchemeColor::Accent2));
        let b = parse_first(r#"<a:schemeClr val="phClr"/>"#, parse_color_element).expect("b");
        assert_eq!(b.spec, ColorSpec::Placeholder);
    }

    #[test]
    fn solid_fill_unwraps_to_its_colour() {
        let f = parse_first(
            r#"<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#,
            parse_fill,
        )
        .expect("fill");
        match f {
            Fill::Solid(c) => assert_eq!(c.resolve(&Office, None), Color::rgb(255, 0, 0)),
            other => panic!("expected solid, got {other:?}"),
        }
    }

    #[test]
    fn no_fill_and_group_fill_are_distinct_from_absent() {
        assert_eq!(
            parse_first(r#"<a:noFill/>"#, parse_fill),
            Some(Fill::NoFill)
        );
        assert_eq!(
            parse_first(r#"<a:grpFill/>"#, parse_fill),
            Some(Fill::Group)
        );
        assert_eq!(parse_first(r#"<a:effectLst/>"#, parse_fill), None);
    }

    #[test]
    fn gradient_stops_are_sorted_and_angles_converted() {
        let f = parse_first(
            r#"<a:gradFill>
                 <a:gsLst>
                   <a:gs pos="100000"><a:srgbClr val="000000"/></a:gs>
                   <a:gs pos="0"><a:srgbClr val="FFFFFF"/></a:gs>
                 </a:gsLst>
                 <a:lin ang="5400000" scaled="1"/>
               </a:gradFill>"#,
            parse_fill,
        )
        .expect("fill");
        match f {
            Fill::Gradient(g) => {
                assert_eq!(g.stops.len(), 2);
                assert_eq!(g.stops[0].pos, 0.0);
                assert_eq!(g.stops[1].pos, 1.0);
                assert_eq!(g.angle_deg, 90.0);
                assert!(g.scaled);
            }
            other => panic!("expected gradient, got {other:?}"),
        }
    }

    #[test]
    fn blip_fill_captures_the_relationship_and_the_crop() {
        let f = parse_first(
            r#"<a:blipFill>
                 <a:blip r:embed="rId7"><a:alphaModFix amt="50000"/></a:blip>
                 <a:srcRect l="10000" t="0" r="20000" b="5000"/>
                 <a:stretch><a:fillRect/></a:stretch>
               </a:blipFill>"#,
            parse_fill,
        )
        .expect("fill");
        match f {
            Fill::Blip(b) => {
                assert_eq!(b.embed_id.as_deref(), Some("rId7"));
                assert_eq!(b.src_rect, [0.1, 0.0, 0.2, 0.05]);
                assert_eq!(b.mode, BlipMode::Stretch);
                assert!((b.alpha - 0.5).abs() < 1e-6);
            }
            other => panic!("expected blip, got {other:?}"),
        }
    }

    #[test]
    fn line_parses_width_dash_join_and_fill() {
        let l = parse_first(
            r#"<a:ln w="28575" cap="rnd">
                 <a:solidFill><a:schemeClr val="accent1"/></a:solidFill>
                 <a:prstDash val="dash"/>
                 <a:round/>
                 <a:tailEnd type="triangle" w="lg" len="lg"/>
               </a:ln>"#,
            parse_line,
        );
        assert_eq!(l.width, Some(28_575));
        assert_eq!(l.cap, Some(LineCapStyle::Round));
        assert_eq!(l.dash, Some(DashStyle::Dash));
        assert_eq!(l.join, Some(LineJoinStyle::Round));
        assert!(matches!(l.fill, Fill::Solid(_)));
        let tail = l.tail.expect("tail");
        assert_eq!(tail.kind, ArrowType::Triangle);
        assert_eq!(tail.width, 4.0);
    }

    #[test]
    fn an_empty_line_element_specifies_nothing() {
        let l = parse_first(r#"<a:ln/>"#, parse_line);
        assert!(
            l.is_empty(),
            "an <a:ln/> with no attributes must stay inheritable"
        );
    }

    #[test]
    fn a_line_with_nofill_is_specified_as_invisible() {
        let l = parse_first(r#"<a:ln><a:noFill/></a:ln>"#, parse_line);
        assert_eq!(l.fill, Fill::NoFill);
        assert!(!l.is_empty());
    }

    #[test]
    fn xfrm_records_offset_extent_rotation_and_flips() {
        let t = parse_first(
            r#"<a:xfrm rot="5400000" flipH="1">
                 <a:off x="914400" y="457200"/>
                 <a:ext cx="1828800" cy="914400"/>
               </a:xfrm>"#,
            parse_xfrm,
        );
        assert_eq!((t.offset_x, t.offset_y), (914_400, 457_200));
        assert_eq!((t.extent_x, t.extent_y), (1_828_800, 914_400));
        assert_eq!(t.rotation, 5_400_000);
        assert!(t.flip_h && !t.flip_v);
        assert!(t.specified);
    }

    #[test]
    fn an_absent_xfrm_is_marked_unspecified_so_it_can_inherit() {
        let t = parse_first(r#"<a:xfrm/>"#, parse_xfrm);
        assert!(!t.specified);
    }

    #[test]
    fn group_xfrm_defaults_a_missing_child_extent_to_its_own() {
        let g = parse_first(
            r#"<a:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="500"/></a:xfrm>"#,
            parse_group_xfrm,
        );
        assert_eq!(g.child_extent_x, 1000);
        assert_eq!(g.child_scale(), (1.0, 1.0));
    }

    #[test]
    fn preset_geometry_keeps_its_adjustments() {
        let g = parse_first(
            r#"<a:prstGeom prst="roundRect"><a:avLst><a:gd name="adj" fmla="val 25000"/></a:avLst></a:prstGeom>"#,
            parse_geometry,
        )
        .expect("geometry");
        match g {
            Geometry::Preset {
                preset,
                adjustments,
            } => {
                assert_eq!(preset, "roundRect");
                assert_eq!(adjustments, vec![("adj".to_string(), 25000.0)]);
            }
            other => panic!("expected preset, got {other:?}"),
        }
    }

    #[test]
    fn custom_geometry_captures_guides_and_path_commands() {
        let g = parse_first(
            r#"<a:custGeom>
                 <a:gdLst><a:gd name="x1" fmla="*/ w 1 2"/></a:gdLst>
                 <a:pathLst>
                   <a:path w="100" h="100">
                     <a:moveTo><a:pt x="0" y="0"/></a:moveTo>
                     <a:lnTo><a:pt x="x1" y="100"/></a:lnTo>
                     <a:cubicBezTo><a:pt x="1" y="2"/><a:pt x="3" y="4"/><a:pt x="5" y="6"/></a:cubicBezTo>
                     <a:close/>
                   </a:path>
                 </a:pathLst>
               </a:custGeom>"#,
            parse_geometry,
        )
        .expect("geometry");
        let Geometry::Custom(c) = g else {
            panic!("expected custom geometry")
        };
        assert_eq!(c.guides.len(), 1);
        assert_eq!(c.paths.len(), 1);
        let p = c.paths.first().expect("path");
        assert_eq!(p.width, Some(100));
        assert_eq!(p.commands.len(), 4);
        assert!(matches!(
            p.commands.first(),
            Some(GeomCommand::MoveTo(_, _))
        ));
        // A guide name is kept unevaluated until the shape's size is known.
        assert!(matches!(
            p.commands.get(1),
            Some(GeomCommand::LineTo(Expr::Name(_), Expr::Literal(_)))
        ));
        assert!(matches!(p.commands.get(3), Some(GeomCommand::Close)));
    }

    #[test]
    fn outer_shadow_keeps_blur_distance_direction_and_colour() {
        let fx = parse_first(
            r#"<a:effectLst>
                 <a:outerShdw blurRad="76200" dist="38100" dir="2700000">
                   <a:srgbClr val="000000"><a:alpha val="40000"/></a:srgbClr>
                 </a:outerShdw>
               </a:effectLst>"#,
            |r, _e, empty| parse_effects(r, empty),
        );
        let s = fx.outer_shadow.expect("shadow");
        assert_eq!(s.blur, 76_200);
        assert_eq!(s.distance, 38_100);
        assert_eq!(s.direction_deg, 45.0);
        assert_eq!(s.color.resolve(&Office, None).a, 102);
    }

    #[test]
    fn unknown_children_are_skipped_without_leaking_grandchildren() {
        // <a:mysteryElement> contains something that looks like a fill; it must not be
        // mistaken for a direct child of <a:ln>.
        let l = parse_first(
            r#"<a:ln w="100"><a:mysteryElement><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill></a:mysteryElement><a:prstDash val="dot"/></a:ln>"#,
            parse_line,
        );
        assert_eq!(l.width, Some(100));
        assert_eq!(
            l.dash,
            Some(DashStyle::Dot),
            "parser must still see the sibling"
        );
        assert!(
            !l.fill.is_specified(),
            "the nested fill must not have been adopted"
        );
    }

    #[test]
    fn style_ref_records_indices_and_their_colours() {
        let s = parse_first(
            r#"<p:style>
                 <a:lnRef idx="2"><a:schemeClr val="accent1"><a:shade val="50000"/></a:schemeClr></a:lnRef>
                 <a:fillRef idx="1"><a:schemeClr val="accent1"/></a:fillRef>
                 <a:effectRef idx="0"><a:schemeClr val="accent1"/></a:effectRef>
                 <a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef>
               </p:style>"#,
            |r, _e, _empty| parse_style_ref(r),
        );
        assert_eq!(s.line_idx, Some(2));
        assert_eq!(s.fill_idx, Some(1));
        assert_eq!(s.effect_idx, Some(0));
        assert_eq!(s.font_kind.as_deref(), Some("minor"));
        let line_color = s.line_color.expect("line colour");
        assert_eq!(line_color.mods, vec![ColorMod::Shade(0.5)]);
    }
}
