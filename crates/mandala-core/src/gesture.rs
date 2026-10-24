//! Gesture engine. Pure and clock-injected, so it is fully unit-testable.
//!
//! Spatial partitioning (from the design doc): the upper 70% of the screen
//! edges belongs to the OS "Back" gesture and is passed through; only the
//! lower 30% edge zones open Panels B/C.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GestureConfig {
    /// Fraction of screen height below which edge swipes belong to Mandala.
    pub edge_zone_top_frac: f32,
    /// Width of the touch-sensitive vertical band at each side edge.
    pub edge_band_px: f32,
    /// Height of the bottom anchor band for swipe-up / hold.
    pub bottom_anchor_px: f32,
    /// Minimum travel to count as a swipe.
    pub swipe_min_px: f32,
    /// Max movement for a press to still count as stationary.
    pub slop_px: f32,
    pub hold_ms: u64,
    pub double_swipe_ms: u64,
}

impl Default for GestureConfig {
    fn default() -> Self {
        Self {
            edge_zone_top_frac: 0.70,
            edge_band_px: 28.0,
            bottom_anchor_px: 56.0,
            swipe_min_px: 56.0,
            slop_px: 12.0,
            hold_ms: 700,
            double_swipe_ms: 500,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// Inward swipe from the left edge, lower 30%.
    OpenPanelB,
    /// Inward swipe from the right edge, lower 30%.
    OpenPanelC,
    /// Inward swipe from an upper-70% edge: owned by the OS, not consumed.
    SystemBackPassthrough,
    /// Swipe up from the bottom anchor.
    SwipeUp,
    /// Two swipe-ups in quick succession.
    DoubleSwipeUp,
    /// Press-and-hold on the bottom anchor.
    HoldBottom,
    /// Generic swipe toward the left (closes the left drawer).
    SwipeLeft,
    /// Generic swipe toward the right (closes the right drawer).
    SwipeRight,
    Tap,
    None,
}

#[derive(Clone, Copy, Debug)]
struct Press {
    x: f32,
    y: f32,
    at_ms: u64,
}

#[derive(Debug)]
pub struct GestureTracker {
    cfg: GestureConfig,
    press: Option<Press>,
    last_swipe_up_ms: Option<u64>,
}

impl GestureTracker {
    pub fn new(cfg: GestureConfig) -> Self {
        Self { cfg, press: None, last_swipe_up_ms: None }
    }

    pub fn config(&self) -> &GestureConfig {
        &self.cfg
    }

    pub fn pointer_down(&mut self, x: f32, y: f32, now_ms: u64) {
        self.press = Some(Press { x, y, at_ms: now_ms });
    }

    /// Classify the gesture that just ended. `w`/`h` are the window size.
    pub fn pointer_up(&mut self, x: f32, y: f32, w: f32, h: f32, now_ms: u64) -> Gesture {
        let Some(p) = self.press.take() else { return Gesture::None };
        let c = self.cfg;
        let (dx, dy) = (x - p.x, y - p.y);
        let dist = (dx * dx + dy * dy).sqrt();
        let held = now_ms.saturating_sub(p.at_ms);
        let in_bottom_anchor = p.y >= h - c.bottom_anchor_px;

        // Stationary press.
        if dist <= c.slop_px {
            if in_bottom_anchor && held >= c.hold_ms {
                return Gesture::HoldBottom;
            }
            return Gesture::Tap;
        }

        let horizontal = dx.abs() >= c.swipe_min_px && dx.abs() > dy.abs() * 2.0;
        let upward = -dy >= c.swipe_min_px && dy.abs() > dx.abs() * 2.0;

        if horizontal {
            let from_left = p.x <= c.edge_band_px;
            let from_right = p.x >= w - c.edge_band_px;
            let in_lower_zone = p.y >= h * c.edge_zone_top_frac;
            if (from_left && dx > 0.0) || (from_right && dx < 0.0) {
                if !in_lower_zone {
                    return Gesture::SystemBackPassthrough;
                }
                return if from_left { Gesture::OpenPanelB } else { Gesture::OpenPanelC };
            }
            return if dx < 0.0 { Gesture::SwipeLeft } else { Gesture::SwipeRight };
        }

        if upward && in_bottom_anchor {
            let double = self
                .last_swipe_up_ms
                .map(|t| now_ms.saturating_sub(t) <= c.double_swipe_ms)
                .unwrap_or(false);
            if double {
                self.last_swipe_up_ms = None;
                return Gesture::DoubleSwipeUp;
            }
            self.last_swipe_up_ms = Some(now_ms);
            return Gesture::SwipeUp;
        }

        Gesture::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 412.0;
    const H: f32 = 915.0;

    fn swipe(t: &mut GestureTracker, from: (f32, f32), to: (f32, f32), t0: u64, t1: u64) -> Gesture {
        t.pointer_down(from.0, from.1, t0);
        t.pointer_up(to.0, to.1, W, H, t1)
    }

    fn tracker() -> GestureTracker {
        GestureTracker::new(GestureConfig::default())
    }

    #[test]
    fn lower_left_edge_opens_panel_b() {
        assert_eq!(swipe(&mut tracker(), (4.0, 800.0), (200.0, 805.0), 0, 120), Gesture::OpenPanelB);
    }

    #[test]
    fn lower_right_edge_opens_panel_c() {
        assert_eq!(swipe(&mut tracker(), (W - 4.0, 700.0), (W - 220.0, 702.0), 0, 120), Gesture::OpenPanelC);
    }

    #[test]
    fn upper_edge_swipes_pass_through_to_os() {
        assert_eq!(swipe(&mut tracker(), (4.0, 300.0), (200.0, 300.0), 0, 120), Gesture::SystemBackPassthrough);
        assert_eq!(swipe(&mut tracker(), (W - 4.0, 100.0), (60.0, 100.0), 0, 120), Gesture::SystemBackPassthrough);
    }

    #[test]
    fn threshold_boundary_is_70_percent() {
        let y_just_above = H * 0.70 - 1.0;
        let y_just_below = H * 0.70 + 1.0;
        assert_eq!(swipe(&mut tracker(), (3.0, y_just_above), (150.0, y_just_above), 0, 100), Gesture::SystemBackPassthrough);
        assert_eq!(swipe(&mut tracker(), (3.0, y_just_below), (150.0, y_just_below), 0, 100), Gesture::OpenPanelB);
    }

    #[test]
    fn outward_swipes_close_drawers() {
        assert_eq!(swipe(&mut tracker(), (200.0, 500.0), (40.0, 500.0), 0, 100), Gesture::SwipeLeft);
        assert_eq!(swipe(&mut tracker(), (150.0, 500.0), (350.0, 505.0), 0, 100), Gesture::SwipeRight);
    }

    #[test]
    fn swipe_up_from_anchor() {
        assert_eq!(swipe(&mut tracker(), (200.0, H - 10.0), (205.0, H - 200.0), 0, 150), Gesture::SwipeUp);
    }

    #[test]
    fn swipe_up_outside_anchor_is_ignored() {
        assert_eq!(swipe(&mut tracker(), (200.0, 400.0), (200.0, 100.0), 0, 150), Gesture::None);
    }

    #[test]
    fn double_swipe_up_within_window() {
        let mut t = tracker();
        assert_eq!(swipe(&mut t, (200.0, H - 10.0), (200.0, H - 200.0), 0, 100), Gesture::SwipeUp);
        assert_eq!(swipe(&mut t, (200.0, H - 10.0), (200.0, H - 200.0), 300, 400), Gesture::DoubleSwipeUp);
        // counter resets afterwards
        assert_eq!(swipe(&mut t, (200.0, H - 10.0), (200.0, H - 200.0), 500, 600), Gesture::SwipeUp);
    }

    #[test]
    fn slow_second_swipe_is_not_double() {
        let mut t = tracker();
        swipe(&mut t, (200.0, H - 10.0), (200.0, H - 200.0), 0, 100);
        assert_eq!(swipe(&mut t, (200.0, H - 10.0), (200.0, H - 200.0), 1500, 1600), Gesture::SwipeUp);
    }

    #[test]
    fn hold_on_bottom_anchor() {
        assert_eq!(swipe(&mut tracker(), (200.0, H - 10.0), (201.0, H - 10.0), 0, 900), Gesture::HoldBottom);
    }

    #[test]
    fn short_press_is_tap_and_hold_elsewhere_is_tap() {
        assert_eq!(swipe(&mut tracker(), (200.0, H - 10.0), (200.0, H - 10.0), 0, 100), Gesture::Tap);
        assert_eq!(swipe(&mut tracker(), (200.0, 300.0), (200.0, 300.0), 0, 2000), Gesture::Tap);
    }

    #[test]
    fn up_without_down_is_none() {
        assert_eq!(tracker().pointer_up(1.0, 1.0, W, H, 10), Gesture::None);
    }
}
