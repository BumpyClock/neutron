use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use windows::{
    Win32::Graphics::{
        DirectComposition::{
            IDCompositionClip, IDCompositionDevice, IDCompositionVisual, IDCompositionVisual3,
        },
        Dxgi::IDXGISwapChain1,
    },
    core::Interface,
};
use windows_numerics::Matrix3x2;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RetainedLayerId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RetainedLayerState {
    pub(crate) order: u32,
    pub(crate) transform: Matrix3x2,
    pub(crate) opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RetainedLayerClip {
    pub(crate) left: f32,
    pub(crate) top: f32,
    pub(crate) right: f32,
    pub(crate) bottom: f32,
    pub(crate) top_left_radius: f32,
    pub(crate) top_right_radius: f32,
    pub(crate) bottom_right_radius: f32,
    pub(crate) bottom_left_radius: f32,
}

pub(crate) enum RetainedLayerContent<'a> {
    SwapChain(&'a IDXGISwapChain1),
}

pub(crate) struct RetainedCompositor {
    comp_device: IDCompositionDevice,
    root_visual: IDCompositionVisual,
    layers: HashMap<RetainedLayerId, RetainedLayer>,
}

struct RetainedLayer {
    visual: IDCompositionVisual,
    visual3: IDCompositionVisual3,
    state: RetainedLayerState,
}

impl RetainedCompositor {
    pub(crate) fn new(comp_device: IDCompositionDevice, root_visual: IDCompositionVisual) -> Self {
        Self {
            comp_device,
            root_visual,
            layers: HashMap::default(),
        }
    }

    pub(crate) fn set_layer(
        &mut self,
        id: RetainedLayerId,
        state: RetainedLayerState,
        content: RetainedLayerContent<'_>,
    ) -> Result<()> {
        if !self.layers.contains_key(&id) {
            let layer = self.create_layer(state, content)?;
            self.layers.insert(id, layer);
            self.rebuild_visual_order()?;
            return Ok(());
        }

        let layer = self.layers.get_mut(&id).expect("layer must exist");
        if layer.state.order != state.order {
            layer.state.order = state.order;
            self.rebuild_visual_order()?;
        }

        let layer = self.layers.get_mut(&id).expect("layer must exist");
        Self::set_layer_content(&layer.visual, content)?;
        Self::set_layer_state(layer, state)?;
        Ok(())
    }

    pub(crate) fn update_layer_state(
        &mut self,
        id: RetainedLayerId,
        state: RetainedLayerState,
    ) -> Result<()> {
        let layer = self
            .layers
            .get_mut(&id)
            .context("updating unknown retained DirectComposition layer")?;

        if layer.state.order != state.order {
            layer.state.order = state.order;
            self.rebuild_visual_order()?;
        }

        let layer = self.layers.get_mut(&id).expect("layer must exist");
        Self::set_layer_state(layer, state)
    }

    pub(crate) fn contains_layers(&self, active_layers: &[RetainedLayerId]) -> bool {
        active_layers.iter().all(|id| self.layers.contains_key(id))
    }

    pub(crate) fn retain_layers(&mut self, active_layers: &[RetainedLayerId]) -> Result<()> {
        let active_layers = active_layers.iter().copied().collect::<HashSet<_>>();
        let previous_len = self.layers.len();
        self.layers.retain(|id, _| active_layers.contains(id));

        if self.layers.len() != previous_len {
            self.rebuild_visual_order()?;
        }

        Ok(())
    }

    pub(crate) fn set_root_clip(&self, clip: Option<RetainedLayerClip>) -> Result<()> {
        let Some(clip) = clip else {
            return unsafe { self.root_visual.SetClip(None::<&IDCompositionClip>) }
                .context("clearing retained DirectComposition clip");
        };

        let rectangle_clip = unsafe { self.comp_device.CreateRectangleClip() }
            .context("creating retained DirectComposition clip")?;
        unsafe {
            rectangle_clip.SetLeft2(clip.left)?;
            rectangle_clip.SetTop2(clip.top)?;
            rectangle_clip.SetRight2(clip.right)?;
            rectangle_clip.SetBottom2(clip.bottom)?;
            rectangle_clip.SetTopLeftRadiusX2(clip.top_left_radius)?;
            rectangle_clip.SetTopLeftRadiusY2(clip.top_left_radius)?;
            rectangle_clip.SetTopRightRadiusX2(clip.top_right_radius)?;
            rectangle_clip.SetTopRightRadiusY2(clip.top_right_radius)?;
            rectangle_clip.SetBottomRightRadiusX2(clip.bottom_right_radius)?;
            rectangle_clip.SetBottomRightRadiusY2(clip.bottom_right_radius)?;
            rectangle_clip.SetBottomLeftRadiusX2(clip.bottom_left_radius)?;
            rectangle_clip.SetBottomLeftRadiusY2(clip.bottom_left_radius)?;
            self.root_visual.SetClip(&rectangle_clip)?;
        }
        Ok(())
    }

    pub(crate) fn commit(&self) -> Result<()> {
        unsafe { self.comp_device.Commit() }.context("committing retained DirectComposition layers")
    }

    fn create_layer(
        &self,
        state: RetainedLayerState,
        content: RetainedLayerContent<'_>,
    ) -> Result<RetainedLayer> {
        let visual = unsafe { self.comp_device.CreateVisual() }
            .context("creating retained DirectComposition visual")?;
        let visual3 = visual
            .cast::<IDCompositionVisual3>()
            .context("IDCompositionVisual3 is required for retained layer opacity")?;

        Self::set_layer_content(&visual, content)?;

        let mut layer = RetainedLayer {
            visual,
            visual3,
            state,
        };
        Self::set_layer_state(&mut layer, state)?;
        Ok(layer)
    }

    fn set_layer_content(
        visual: &IDCompositionVisual,
        content: RetainedLayerContent<'_>,
    ) -> Result<()> {
        unsafe {
            match content {
                RetainedLayerContent::SwapChain(swap_chain) => visual.SetContent(swap_chain),
            }
        }
        .context("setting retained DirectComposition layer content")
    }

    fn set_layer_state(layer: &mut RetainedLayer, state: RetainedLayerState) -> Result<()> {
        unsafe {
            layer.visual.SetTransform2(&state.transform)?;
            layer.visual3.SetOpacity2(state.opacity.clamp(0.0, 1.0))?;
        }

        layer.state = state;
        Ok(())
    }

    fn rebuild_visual_order(&mut self) -> Result<()> {
        let mut ordered_layers = self
            .layers
            .iter()
            .map(|(id, layer)| (*id, layer.state.order, layer.visual.clone()))
            .collect::<Vec<_>>();
        ordered_layers.sort_by_key(|(id, order, _)| (*order, id.0));

        unsafe {
            self.root_visual
                .RemoveAllVisuals()
                .context("clearing retained DirectComposition visuals")?;

            let mut previous_visual = None;
            for (_, _, visual) in ordered_layers {
                self.root_visual
                    .AddVisual(&visual, previous_visual.is_some(), previous_visual.as_ref())
                    .context("adding retained DirectComposition visual")?;
                previous_visual = Some(visual);
            }
        }

        Ok(())
    }
}
