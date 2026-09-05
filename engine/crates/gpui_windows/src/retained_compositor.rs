use std::{cmp::Reverse, collections::HashMap, fmt, ops::Range};

use gpui::{
    Bounds, ContentMask, GlobalElementId, Point, RetainedLayer, RetainedLayerContentRevision,
    ScaledPixels, Scene, TransformationMatrix, point,
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RetainedLayerKey {
    pub id: GlobalElementId,
    pub occurrence: usize,
}

#[derive(Debug, PartialEq)]
pub(crate) enum RetainedCompositionError {
    InvalidPaintRange,
    OverlappingPaintRanges,
    NonFiniteGeometry,
    TextureTooLarge,
}

impl fmt::Display for RetainedCompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPaintRange => "retained layer has an invalid paint range",
            Self::OverlappingPaintRanges => "retained layer paint ranges cross without containment",
            Self::NonFiniteGeometry => "retained layer has non-finite geometry or opacity",
            Self::TextureTooLarge => "retained layer exceeds the Direct3D texture size limit",
        })
    }
}

impl std::error::Error for RetainedCompositionError {}

pub(crate) struct RetainedScenePlan {
    pub keys: Vec<RetainedLayerKey>,
    pub roots: Vec<usize>,
    pub children: Vec<Vec<usize>>,
}

impl RetainedScenePlan {
    pub fn new(scene: &Scene) -> Result<Self, RetainedCompositionError> {
        let mut occurrences = HashMap::new();
        let keys = scene
            .retained_layers
            .iter()
            .map(|layer| {
                let occurrence = occurrences.entry(layer.id.clone()).or_insert(0);
                let key = RetainedLayerKey {
                    id: layer.id.clone(),
                    occurrence: *occurrence,
                };
                *occurrence += 1;
                key
            })
            .collect();
        let mut ordered = (0..scene.retained_layers.len()).collect::<Vec<_>>();
        // Descriptors are emitted after their children, including equal-range wrappers.
        ordered.sort_by_key(|&index| {
            let range = &scene.retained_layers[index].paint_range;
            (range.start, Reverse(range.end), Reverse(index))
        });
        let mut roots = Vec::new();
        let mut children = vec![Vec::new(); scene.retained_layers.len()];
        let mut stack: Vec<usize> = Vec::new();
        for index in ordered {
            let layer = &scene.retained_layers[index];
            let range = &layer.paint_range;
            if range.start > range.end || range.end > scene.paint_operation_count() {
                return Err(RetainedCompositionError::InvalidPaintRange);
            }
            if !finite_layer(layer) {
                return Err(RetainedCompositionError::NonFiniteGeometry);
            }
            if range.is_empty() {
                continue;
            }
            while stack
                .last()
                .is_some_and(|&parent| scene.retained_layers[parent].paint_range.end <= range.start)
            {
                stack.pop();
            }
            if let Some(&parent) = stack.last() {
                if range.end > scene.retained_layers[parent].paint_range.end {
                    return Err(RetainedCompositionError::OverlappingPaintRanges);
                }
                children[parent].push(index);
            } else {
                roots.push(index);
            }
            stack.push(index);
        }
        Ok(Self {
            keys,
            roots,
            children,
        })
    }

    pub fn descendants(&self, index: usize, result: &mut Vec<usize>) {
        for &child in &self.children[index] {
            result.push(child);
            self.descendants(child, result);
        }
    }
}

fn finite_layer(layer: &RetainedLayer) -> bool {
    let bounds = [
        layer.bounds,
        layer.content_mask.bounds,
        layer.content_mask.rounded_bounds,
    ];
    bounds.into_iter().all(|bounds| {
        [
            bounds.origin.x.0,
            bounds.origin.y.0,
            bounds.size.width.0,
            bounds.size.height.0,
        ]
        .into_iter()
        .all(f32::is_finite)
    }) && layer
        .transform
        .rotation_scale
        .iter()
        .flatten()
        .chain(layer.transform.translation.iter())
        .copied()
        .all(f32::is_finite)
        && layer.opacity.is_finite()
        && [
            layer.content_mask.corner_radii.top_left.0,
            layer.content_mask.corner_radii.top_right.0,
            layer.content_mask.corner_radii.bottom_right.0,
            layer.content_mask.corner_radii.bottom_left.0,
        ]
        .into_iter()
        .all(f32::is_finite)
}

#[derive(Clone, PartialEq)]
pub(crate) struct RetainedLayerInput {
    revision: RetainedLayerContentRevision,
    bounds: Bounds<ScaledPixels>,
    mask: ContentMask<ScaledPixels>,
    descendants: Vec<RetainedLayer>,
}

impl RetainedLayerInput {
    pub fn new(scene: &Scene, plan: &RetainedScenePlan, index: usize) -> (Self, bool) {
        let layer = &scene.retained_layers[index];
        let mut descendants = Vec::new();
        plan.descendants(index, &mut descendants);
        let mut dirty = layer.content_dirty;
        let descendants = descendants
            .into_iter()
            .map(|index| {
                let mut child = scene.retained_layers[index].clone();
                dirty |= child.content_dirty;
                child.content_dirty = false;
                child.paint_range = child.paint_range.start - layer.paint_range.start
                    ..child.paint_range.end - layer.paint_range.start;
                child
            })
            .collect();
        (
            Self {
                revision: layer.content_revision,
                bounds: layer.bounds,
                mask: layer.content_mask.clone(),
                descendants,
            },
            dirty,
        )
    }
}

pub(crate) fn paint_segments(
    range: Range<usize>,
    children: &[usize],
    layers: &[RetainedLayer],
) -> Vec<(Range<usize>, Option<usize>)> {
    let mut result = Vec::with_capacity(children.len() + 1);
    let mut cursor = range.start;
    for &child in children {
        let child_range = &layers[child].paint_range;
        result.push((cursor..child_range.start, Some(child)));
        cursor = child_range.end;
    }
    result.push((cursor..range.end, None));
    result
}

pub(crate) fn retained_backdrop_layers(scene: &Scene) -> Vec<bool> {
    let mut dependent = vec![false; scene.retained_layers.len()];
    if scene.backdrop_blurs.is_empty() {
        return dependent;
    }
    for (index, layer) in scene.retained_layers.iter().enumerate() {
        if !scene
            .clone_paint_range(layer.paint_range.clone())
            .backdrop_blurs
            .is_empty()
        {
            dependent[index] = true;
        }
    }
    dependent
}

pub(crate) fn direct_backdrop_layer(layer: &RetainedLayer, dependent: bool) -> bool {
    dependent && layer.transform.is_unit() && layer.opacity == 1.0
}

pub(crate) fn composite_children(
    scene: &Scene,
    plan: &RetainedScenePlan,
    dependent: &[bool],
    children: &[usize],
) -> Vec<usize> {
    let mut result = Vec::new();
    for &child in children {
        if direct_backdrop_layer(&scene.retained_layers[child], dependent[child]) {
            result.extend(composite_children(
                scene,
                plan,
                dependent,
                &plan.children[child],
            ));
        } else {
            result.push(child);
        }
    }
    result
}

pub(crate) fn retained_raster_bounds(
    scene: &Scene,
    plan: &RetainedScenePlan,
    index: usize,
) -> Option<Bounds<ScaledPixels>> {
    let geometry = retained_raster_geometry(scene, plan, index)?;
    let padding = geometry.padding.map(ScaledPixels);
    Some(Bounds::from_corners(
        geometry.content.origin - padding,
        geometry.content.bottom_right() + padding,
    ))
}

struct RetainedRasterGeometry {
    content: Bounds<ScaledPixels>,
    padding: Point<f32>,
}

fn retained_raster_geometry(
    scene: &Scene,
    plan: &RetainedScenePlan,
    index: usize,
) -> Option<RetainedRasterGeometry> {
    let layer = &scene.retained_layers[index];
    if layer.opacity <= 0.0 {
        return None;
    }
    let mut content = None;
    let mut padding = point(0.0_f32, 0.0_f32);
    for (range, _) in paint_segments(
        layer.paint_range.clone(),
        &plan.children[index],
        &scene.retained_layers,
    ) {
        let direct = scene.clone_paint_range(range);
        if let Some(bounds) = content_bounds(&direct) {
            union_visible_bounds(&mut content, bounds);
        }
        for blur in &direct.backdrop_blurs {
            if !blur.bounds.intersect(&blur.content_mask.bounds).is_empty() {
                let support = gpui::BackdropBlurPlan::padding(blur.blur_radius.0);
                padding.x = padding.x.max(support);
                padding.y = padding.y.max(support);
            }
        }
    }
    for &child in &plan.children[index] {
        let Some(geometry) = retained_raster_geometry(scene, plan, child) else {
            continue;
        };
        let child = &scene.retained_layers[child];
        let visible = transformed_bounds(geometry.content, child.transform)
            .intersect(&child.content_mask.bounds);
        if visible.is_empty() {
            continue;
        }
        union_visible_bounds(&mut content, visible);
        // Visible blur output needs parent samples outside its destination clip.
        let matrix = child.transform.rotation_scale;
        padding.x = padding
            .x
            .max(matrix[0][0].abs() * geometry.padding.x + matrix[0][1].abs() * geometry.padding.y);
        padding.y = padding
            .y
            .max(matrix[1][0].abs() * geometry.padding.x + matrix[1][1].abs() * geometry.padding.y);
    }
    let mut content = content?;
    if layer.transform.is_unit() {
        content = content.intersect(&layer.content_mask.bounds);
    }
    if content.is_empty()
        || transformed_bounds(content, layer.transform)
            .intersect(&layer.content_mask.bounds)
            .is_empty()
    {
        return None;
    }
    Some(RetainedRasterGeometry { content, padding })
}

fn union_visible_bounds(result: &mut Option<Bounds<ScaledPixels>>, bounds: Bounds<ScaledPixels>) {
    if !bounds.is_empty() {
        *result = Some(result.map_or(bounds, |previous| previous.union(&bounds)));
    }
}

pub(crate) fn transformed_bounds(
    bounds: Bounds<ScaledPixels>,
    transform: TransformationMatrix,
) -> Bounds<ScaledPixels> {
    let corners = [
        bounds.origin,
        point(bounds.right(), bounds.top()),
        bounds.bottom_right(),
        point(bounds.left(), bounds.bottom()),
    ]
    .map(|point| transform.apply_scaled(point));
    let mut min = corners[0];
    let mut max = corners[0];
    for corner in corners {
        min.x = min.x.min(corner.x);
        min.y = min.y.min(corner.y);
        max.x = max.x.max(corner.x);
        max.y = max.y.max(corner.y);
    }
    Bounds::from_corners(min, max)
}

pub(crate) fn backdrop_projection_uvs(
    bounds: Bounds<ScaledPixels>,
    transform: TransformationMatrix,
    source_bounds: Bounds<ScaledPixels>,
) -> [Point<f32>; 4] {
    [
        bounds.origin,
        point(bounds.right(), bounds.top()),
        point(bounds.left(), bounds.bottom()),
        bounds.bottom_right(),
    ]
    .map(|position| {
        let position = transform.apply_scaled(position) - source_bounds.origin;
        point(
            position.x.0 / source_bounds.size.width.0.max(1.0),
            position.y.0 / source_bounds.size.height.0.max(1.0),
        )
    })
}

pub(crate) fn texture_bounds(
    bounds: Bounds<ScaledPixels>,
    limit: u32,
) -> Result<Bounds<ScaledPixels>, RetainedCompositionError> {
    let origin = bounds.origin.map(|value| value.floor());
    let end = bounds.bottom_right().map(|value| value.ceil());
    let bounds = Bounds::from_corners(origin, end);
    let width = bounds.size.width.0;
    let height = bounds.size.height.0;
    if !width.is_finite() || !height.is_finite() {
        return Err(RetainedCompositionError::NonFiniteGeometry);
    }
    if width > limit as f32 || height > limit as f32 {
        return Err(RetainedCompositionError::TextureTooLarge);
    }
    Ok(bounds)
}

pub(crate) fn content_bounds(scene: &Scene) -> Option<Bounds<ScaledPixels>> {
    let mut result: Option<Bounds<ScaledPixels>> = None;
    let mut include = |bounds: Bounds<ScaledPixels>| {
        if !bounds.is_empty() {
            result = Some(result.map_or(bounds, |previous| previous.union(&bounds)));
        }
    };
    for quad in &scene.quads {
        include(quad.bounds.intersect(&quad.content_mask.bounds));
    }
    for blur in &scene.backdrop_blurs {
        include(blur.bounds.intersect(&blur.content_mask.bounds));
    }
    for shadow in &scene.shadows {
        let bounds = if shadow.inset != 0 {
            shadow.element_bounds
        } else {
            shadow.bounds
        };
        include(bounds.intersect(&shadow.content_mask.bounds));
    }
    for path in &scene.paths {
        include(path.clipped_bounds().dilate(ScaledPixels(1.0)));
    }
    for underline in &scene.underlines {
        include(underline.bounds.intersect(&underline.content_mask.bounds));
    }
    for sprite in &scene.monochrome_sprites {
        include(
            transformed_bounds(sprite.bounds, sprite.transformation)
                .intersect(&sprite.content_mask.bounds),
        );
    }
    for sprite in &scene.subpixel_sprites {
        include(
            transformed_bounds(sprite.bounds, sprite.transformation)
                .intersect(&sprite.content_mask.bounds),
        );
    }
    for sprite in &scene.polychrome_sprites {
        include(sprite.bounds.intersect(&sprite.content_mask.bounds));
    }
    for surface in &scene.surfaces {
        include(surface.bounds.intersect(&surface.content_mask.bounds));
    }
    result
}

pub(crate) fn localize_mask(mask: &mut ContentMask<ScaledPixels>, origin: Point<ScaledPixels>) {
    mask.bounds = mask.bounds - origin;
    mask.rounded_bounds = mask.rounded_bounds - origin;
}

fn localize_transform(
    transform: TransformationMatrix,
    origin: Point<ScaledPixels>,
) -> TransformationMatrix {
    let translation = transform.apply_scaled(origin) - origin;
    TransformationMatrix {
        rotation_scale: transform.rotation_scale,
        translation: [translation.x.0, translation.y.0],
    }
}

pub(crate) fn localize_scene(scene: &mut Scene, origin: Point<ScaledPixels>) {
    if origin == Point::default() {
        return;
    }
    for quad in &mut scene.quads {
        quad.bounds = quad.bounds - origin;
        localize_mask(&mut quad.content_mask, origin);
    }
    for shadow in &mut scene.shadows {
        shadow.bounds = shadow.bounds - origin;
        shadow.element_bounds = shadow.element_bounds - origin;
        localize_mask(&mut shadow.content_mask, origin);
    }
    for blur in &mut scene.backdrop_blurs {
        blur.bounds = blur.bounds - origin;
        localize_mask(&mut blur.content_mask, origin);
    }
    for path in &mut scene.paths {
        path.bounds = path.bounds - origin;
        localize_mask(&mut path.content_mask, origin);
        for vertex in &mut path.vertices {
            vertex.xy_position -= origin;
            localize_mask(&mut vertex.content_mask, origin);
        }
    }
    for underline in &mut scene.underlines {
        underline.bounds = underline.bounds - origin;
        localize_mask(&mut underline.content_mask, origin);
    }
    for sprite in &mut scene.monochrome_sprites {
        sprite.bounds = sprite.bounds - origin;
        localize_mask(&mut sprite.content_mask, origin);
        sprite.transformation = localize_transform(sprite.transformation, origin);
    }
    for sprite in &mut scene.subpixel_sprites {
        sprite.bounds = sprite.bounds - origin;
        localize_mask(&mut sprite.content_mask, origin);
        sprite.transformation = localize_transform(sprite.transformation, origin);
    }
    for sprite in &mut scene.polychrome_sprites {
        sprite.bounds = sprite.bounds - origin;
        localize_mask(&mut sprite.content_mask, origin);
    }
    for surface in &mut scene.surfaces {
        surface.bounds = surface.bounds - origin;
        localize_mask(&mut surface.content_mask, origin);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Quad, RetainedLayerContentRevision, Shadow, size};

    fn layer(id: &'static str, range: Range<usize>) -> RetainedLayer {
        let mut global_id = GlobalElementId::default();
        *global_id = vec![id.into()].into();
        let bounds = Bounds::new(
            point(ScaledPixels(10.0), ScaledPixels(20.0)),
            size(ScaledPixels(30.0), ScaledPixels(40.0)),
        );
        RetainedLayer {
            id: global_id,
            content_revision: RetainedLayerContentRevision(1),
            content_dirty: false,
            bounds,
            content_mask: ContentMask::new(bounds),
            transform: TransformationMatrix::unit(),
            opacity: 1.0,
            paint_range: range,
        }
    }

    fn scene(layers: Vec<RetainedLayer>) -> Scene {
        let mut scene = Scene::default();
        for _ in 0..8 {
            scene.insert_primitive(Quad {
                bounds: layers[0].bounds,
                content_mask: layers[0].content_mask.clone(),
                ..Default::default()
            });
        }
        scene.retained_layers = layers;
        scene
    }

    #[test]
    fn nested_and_equal_ranges_preserve_paint_order() {
        let scene = scene(vec![
            layer("child", 2..4),
            layer("wrapper", 2..4),
            layer("parent", 1..5),
            layer("sibling", 6..7),
        ]);
        let plan = RetainedScenePlan::new(&scene).unwrap();
        assert_eq!(plan.roots, vec![2, 3]);
        assert_eq!(plan.children[2], vec![1]);
        assert_eq!(plan.children[1], vec![0]);
        assert_eq!(
            paint_segments(0..8, &plan.roots, &scene.retained_layers),
            vec![(0..1, Some(2)), (5..6, Some(3)), (7..8, None)]
        );
    }

    #[test]
    fn crossed_ranges_fail_before_draw() {
        let scene = scene(vec![layer("first", 1..4), layer("second", 3..6)]);
        assert!(matches!(
            RetainedScenePlan::new(&scene),
            Err(RetainedCompositionError::OverlappingPaintRanges)
        ));
    }

    #[test]
    fn cache_ignores_own_compositor_properties_but_tracks_descendants() {
        let mut scene = scene(vec![layer("child", 2..4), layer("parent", 1..5)]);
        let plan = RetainedScenePlan::new(&scene).unwrap();
        let (before, dirty) = RetainedLayerInput::new(&scene, &plan, 1);
        assert!(!dirty);
        scene.retained_layers[1].opacity = 0.5;
        scene.retained_layers[1].transform.translation = [50.0, 0.0];
        assert!(before == RetainedLayerInput::new(&scene, &plan, 1).0);
        scene.retained_layers[0].opacity = 0.25;
        assert!(before != RetainedLayerInput::new(&scene, &plan, 1).0);
        scene.retained_layers[0].content_dirty = true;
        assert!(RetainedLayerInput::new(&scene, &plan, 1).1);
    }

    #[test]
    fn duplicate_ids_have_distinct_cache_entries() {
        let scene = scene(vec![layer("same", 1..3), layer("same", 4..6)]);
        let plan = RetainedScenePlan::new(&scene).unwrap();
        assert_ne!(plan.keys[0], plan.keys[1]);
    }

    #[test]
    fn transformed_texture_bounds_use_global_coordinates_and_ceil_extents() {
        let bounds = Bounds::new(
            point(ScaledPixels(10.25), ScaledPixels(20.5)),
            size(ScaledPixels(30.5), ScaledPixels(40.25)),
        );
        let transform = TransformationMatrix {
            rotation_scale: [[0.0, -1.0], [1.0, 0.0]],
            translation: [100.0, 2.0],
        };
        let bounds = texture_bounds(transformed_bounds(bounds, transform), 16384).unwrap();
        assert_eq!(bounds.origin, point(ScaledPixels(39.0), ScaledPixels(12.0)));
        assert_eq!(bounds.size, size(ScaledPixels(41.0), ScaledPixels(31.0)));
        assert!(matches!(
            texture_bounds(bounds, 32),
            Err(RetainedCompositionError::TextureTooLarge)
        ));
    }

    #[test]
    fn localized_inset_shadow_bounds_and_rounded_mask_stay_aligned() {
        let original = layer("shadow", 0..1).bounds;
        let mut scene = Scene::default();
        scene.shadows.push(Shadow {
            order: 0,
            blur_radius: ScaledPixels(2.0),
            bounds: original.dilate(ScaledPixels(2.0)),
            element_bounds: original,
            inset: 1,
            pad: 0,
            color: Default::default(),
            corner_radii: Default::default(),
            element_corner_radii: Default::default(),
            content_mask: ContentMask::rounded(original, gpui::Corners::all(ScaledPixels(4.0))),
        });
        assert_eq!(content_bounds(&scene), Some(original));
        localize_scene(&mut scene, original.origin);
        assert_eq!(scene.shadows[0].element_bounds.origin, Point::default());
        assert_eq!(
            scene.shadows[0].content_mask.rounded_bounds.origin,
            Point::default()
        );
        assert_eq!(
            scene.shadows[0].bounds.origin,
            point(ScaledPixels(-2.0), ScaledPixels(-2.0))
        );
    }

    #[test]
    fn localized_transform_preserves_global_rotation_and_translation() {
        let origin = point(ScaledPixels(10.0), ScaledPixels(20.0));
        let transform = TransformationMatrix {
            rotation_scale: [[0.0, -1.0], [1.0, 0.0]],
            translation: [100.0, 5.0],
        };
        let position = point(ScaledPixels(17.0), ScaledPixels(29.0));
        assert_eq!(
            localize_transform(transform, origin).apply_scaled(position - origin),
            transform.apply_scaled(position) - origin,
        );
    }

    #[test]
    fn backdrop_dependency_tracks_ancestors_without_rejecting_compositor_properties() {
        let mut scene = Scene::default();
        let mut retained = layer("blur", 0..1);
        scene.insert_primitive(gpui::BackdropBlur {
            order: 0,
            pad: 0,
            bounds: retained.bounds,
            content_mask: retained.content_mask.clone(),
            corner_radii: Default::default(),
            blur_radius: ScaledPixels(4.0),
            source_origin_x: 0.0,
            source_origin_y: 0.0,
            source_width: 0.0,
            source_height: 0.0,
            opacity: 1.0,
        });
        scene.retained_layers.push(retained.clone());
        assert_eq!(retained_backdrop_layers(&scene), vec![true]);
        scene.retained_layers[0].opacity = 0.5;
        assert_eq!(retained_backdrop_layers(&scene), vec![true]);
        assert!(!direct_backdrop_layer(&scene.retained_layers[0], true));
        retained.transform.translation = [1.0, 0.0];
        scene.retained_layers[0] = retained;
        assert_eq!(retained_backdrop_layers(&scene), vec![true]);
        assert!(!direct_backdrop_layer(&scene.retained_layers[0], true));
        scene.retained_layers.clear();
        assert!(retained_backdrop_layers(&scene).is_empty());
    }

    #[test]
    fn invalid_descriptors_fail_before_geometry_or_gpu_work() {
        let mut scene = scene(vec![layer("invalid", 0..9)]);
        assert!(matches!(
            RetainedScenePlan::new(&scene),
            Err(RetainedCompositionError::InvalidPaintRange)
        ));
        scene.retained_layers[0].paint_range = 0..8;
        scene.retained_layers[0].transform.translation[0] = f32::NAN;
        assert!(matches!(
            RetainedScenePlan::new(&scene),
            Err(RetainedCompositionError::NonFiniteGeometry)
        ));
    }

    #[test]
    fn identity_backdrop_wrappers_do_not_split_snapshot_batches() {
        let scene = scene(vec![
            layer("first-blur", 1..3),
            layer("second-blur", 3..5),
            layer("parent", 1..5),
        ]);
        let plan = RetainedScenePlan::new(&scene).unwrap();
        let children = composite_children(&scene, &plan, &[true, true, true], &plan.roots);
        assert!(children.is_empty());
        assert_eq!(
            paint_segments(0..8, &children, &scene.retained_layers),
            vec![(0..8, None)]
        );
    }

    #[test]
    fn backdrop_projection_uses_transformed_positions_in_parent_coordinates() {
        let bounds = Bounds::new(
            point(ScaledPixels(10.0), ScaledPixels(20.0)),
            size(ScaledPixels(30.0), ScaledPixels(40.0)),
        );
        let parent = Bounds::new(
            point(ScaledPixels(4.0), ScaledPixels(8.0)),
            size(ScaledPixels(100.0), ScaledPixels(200.0)),
        );
        let transform = TransformationMatrix {
            rotation_scale: [[0.0, -2.0], [1.0, 0.0]],
            translation: [144.0, 18.0],
        };
        assert_eq!(
            backdrop_projection_uvs(bounds, transform, parent),
            [
                point(1.0, 0.1),
                point(1.0, 0.25),
                point(0.2, 0.1),
                point(0.2, 0.25),
            ]
        );
    }

    #[test]
    fn nested_backdrop_targets_include_transformed_kernel_support() {
        let mut child = layer("blur", 0..1);
        child.transform.rotation_scale[0][0] = 2.0;
        child.transform.translation = [100.0, 0.0];
        child.content_mask = ContentMask::new(Bounds::new(
            Point::default(),
            size(ScaledPixels(200.0), ScaledPixels(100.0)),
        ));
        let mut parent = layer("parent", 0..1);
        parent.content_mask = child.content_mask.clone();
        let mut scene = Scene::default();
        scene.insert_primitive(gpui::BackdropBlur {
            order: 0,
            pad: 0,
            bounds: child.bounds,
            content_mask: child.content_mask.clone(),
            corner_radii: Default::default(),
            blur_radius: ScaledPixels(12.0),
            source_origin_x: 0.0,
            source_origin_y: 0.0,
            source_width: 0.0,
            source_height: 0.0,
            opacity: 1.0,
        });
        scene.retained_layers = vec![child, parent];
        let plan = RetainedScenePlan::new(&scene).unwrap();
        assert_eq!(
            retained_raster_bounds(&scene, &plan, 0).unwrap().origin,
            point(ScaledPixels(-4.0), ScaledPixels(6.0))
        );
        assert_eq!(
            retained_raster_bounds(&scene, &plan, 1).unwrap().right(),
            ScaledPixels(208.0)
        );
        assert_eq!(retained_backdrop_layers(&scene), vec![true, true]);
    }

    #[test]
    fn parent_bounds_exclude_child_source_and_fully_clipped_output() {
        let mut child = layer("child", 1..2);
        child.content_mask = ContentMask::new(Bounds::new(
            Point::default(),
            size(ScaledPixels(64.0), ScaledPixels(64.0)),
        ));
        let mut parent = layer("parent", 1..2);
        parent.content_mask = child.content_mask.clone();
        let mut scene = scene(vec![child, parent]);
        for translation in [-20_000.0, 20_000.0] {
            scene.retained_layers[0].transform.translation[0] = translation;
            let plan = RetainedScenePlan::new(&scene).unwrap();
            assert_eq!(retained_raster_bounds(&scene, &plan, 0), None);
            assert_eq!(retained_raster_bounds(&scene, &plan, 1), None);
        }
        scene.retained_layers[0].transform.translation[0] = 40.0;
        let plan = RetainedScenePlan::new(&scene).unwrap();
        let parent = retained_raster_bounds(&scene, &plan, 1).unwrap();
        assert_eq!(parent.origin, point(ScaledPixels(50.0), ScaledPixels(20.0)));
        assert_eq!(parent.size, size(ScaledPixels(14.0), ScaledPixels(40.0)));
    }
}
