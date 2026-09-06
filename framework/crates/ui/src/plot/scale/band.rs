// @reference: https://d3js.org/d3-scale/band

use itertools::Itertools;
use num_traits::Zero;

use super::Scale;

#[derive(Clone)]
pub struct ScaleBand<T> {
    domain: Vec<T>,
    range_diff: f32,
    avg_width: f32,
    padding_inner: f32,
    padding_outer: f32,
}

impl<T> ScaleBand<T> {
    /// Create a scale with one band per distinct value, in first-occurrence order.
    pub fn new(domain: Vec<T>, range: Vec<f32>) -> Self
    where
        T: PartialEq,
    {
        let mut unique = Vec::with_capacity(domain.len());
        for value in domain {
            if !unique.contains(&value) {
                unique.push(value);
            }
        }
        let domain = unique;
        let len = domain.len() as f32;
        let range_diff = range
            .iter()
            .minmax()
            .into_option()
            .map_or(0., |(min, max)| max - min);

        Self {
            domain,
            range_diff,
            avg_width: if len.is_zero() { 0. } else { range_diff / len },
            padding_inner: 0.,
            padding_outer: 0.,
        }
    }

    /// Get the width of the band.
    pub fn band_width(&self) -> f32 {
        (self.avg_width * (1. - self.padding_inner)).min(30.)
    }

    /// Set the padding inner of the band.
    pub fn padding_inner(mut self, padding_inner: f32) -> Self {
        self.padding_inner = padding_inner;
        self
    }

    /// Set the padding outer of the band.
    pub fn padding_outer(mut self, padding_outer: f32) -> Self {
        self.padding_outer = padding_outer;
        self
    }

    /// Get the ratio of the band.
    fn ratio(&self) -> f32 {
        1. + self.padding_inner / (self.domain.len() - 1) as f32
    }

    /// Get the average width of the band for display.
    fn display_avg_width(&self) -> f32 {
        let padding_outer_width = self.avg_width * self.padding_outer;
        (self.range_diff - padding_outer_width * 2.) / self.domain.len() as f32
    }
}

impl<T> Scale<T> for ScaleBand<T>
where
    T: PartialEq,
{
    fn tick(&self, value: &T) -> Option<f32> {
        let index = self.domain.iter().position(|v| v == value)?;
        let domain_len = self.domain.len();

        // When there's only one element, place it in the center.
        if domain_len == 1 {
            return Some((self.range_diff - self.band_width()) / 2.);
        }

        let avg_width = self.display_avg_width();
        let padding_outer_width = self.avg_width * self.padding_outer;
        Some(index as f32 * avg_width * self.ratio() + padding_outer_width)
    }

    fn least_index(&self, tick: f32) -> usize {
        if self.domain.is_empty() {
            return 0;
        }

        let domain_len = self.domain.len();

        // Handle single element case
        if domain_len == 1 {
            return 0;
        }

        let avg_width = self.display_avg_width();
        let padding_outer_width = self.avg_width * self.padding_outer;
        let adjusted_tick = tick - padding_outer_width;
        let index = (adjusted_tick / (avg_width * self.ratio())).round() as i32;

        (index.max(0) as usize).min(domain_len.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_band() {
        let scale = ScaleBand::new(vec![1, 2, 3], vec![0., 90.]);
        assert_eq!(scale.tick(&1), Some(0.));
        assert_eq!(scale.tick(&2), Some(30.));
        assert_eq!(scale.tick(&3), Some(60.));
        assert_eq!(scale.band_width(), 30.);
    }

    #[test]
    fn test_scale_band_dedup() {
        let scale = ScaleBand::new(vec![3, 1, 3, 2, 1, 2], vec![0., 90.]);
        assert_eq!(scale.tick(&3), Some(0.));
        assert_eq!(scale.tick(&1), Some(30.));
        assert_eq!(scale.tick(&2), Some(60.));
        assert_eq!(scale.tick(&4), None);
        assert_eq!(scale.band_width(), 30.);
        assert_eq!(scale.least_index(-30.), 0);
        assert_eq!(scale.least_index(30.), 1);
        assert_eq!(scale.least_index(60.), 2);
        assert_eq!(scale.least_index(150.), 2);
    }

    #[test]
    fn test_scale_band_dedup_with_padding() {
        let scale = ScaleBand::new(vec![3, 1, 3, 2, 1, 2], vec![0., 90.])
            .padding_inner(0.5)
            .padding_outer(0.5);
        assert_eq!(scale.band_width(), 15.);
        assert_eq!(scale.tick(&3), Some(15.));
        assert_eq!(scale.tick(&1), Some(40.));
        assert_eq!(scale.tick(&2), Some(65.));
        assert_eq!(scale.least_index(15.), 0);
        assert_eq!(scale.least_index(40.), 1);
        assert_eq!(scale.least_index(65.), 2);
    }

    #[test]
    fn test_scale_band_dedup_single_partial_eq_value() {
        let scale = ScaleBand::new(vec![1.5_f32, 1.5, 1.5], vec![0., 90.]);
        assert_eq!(scale.band_width(), 30.);
        assert_eq!(scale.tick(&1.5), Some(30.));
        assert_eq!(scale.tick(&2.5), None);
        assert_eq!(scale.least_index(-30.), 0);
        assert_eq!(scale.least_index(30.), 0);
        assert_eq!(scale.least_index(150.), 0);
    }

    #[test]
    fn test_scale_band_zero() {
        let scale = ScaleBand::new(vec![], vec![0., 90.]);
        assert_eq!(scale.tick(&1), None);
        assert_eq!(scale.tick(&2), None);
        assert_eq!(scale.tick(&3), None);
        assert_eq!(scale.band_width(), 0.);

        let scale = ScaleBand::new(vec![1, 2, 3], vec![]);
        assert_eq!(scale.tick(&1), Some(0.));
        assert_eq!(scale.tick(&2), Some(0.));
        assert_eq!(scale.tick(&3), Some(0.));
        assert_eq!(scale.band_width(), 0.);
    }
}
