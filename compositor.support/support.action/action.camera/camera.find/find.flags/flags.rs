use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct WindowFinderFlags: u32 {
        // Origin selection (fallback order, evaluated top-down)
        const ORIGIN_FOCUSED_VISIBLE       = 1 << 0;
        const ORIGIN_FOCUSED               = 1 << 1;
        const ORIGIN_VISIBLE               = 1 << 2;
        /// Pick the visible window with the largest visible-area-fraction
        /// (visible area / total area). The window LEAST cut off wins.
        const ORIGIN_MOST_VISIBLE_AREA     = 1 << 17;
        /// Pick the visible window whose center is closest to viewport center.
        const ORIGIN_MOST_CENTERED         = 1 << 18;

        // Base-phase pass enablers
        const RAYCAST_BASE                 = 1 << 3;
        const RAYCAST_SCREEN_LOW           = 1 << 4;
        const RAYCAST_SCREEN_HIGH          = 1 << 5;
        const RAYCAST_SCREEN_EXTRA         = 1 << 6;
        const RAYCAST_ALL                  = 1 << 7;

        // Cycling enablers (one cycling pass per group)
        const RAYCAST_CYCLING_BASE         = 1 << 8;
        /// Enables the one cycling pass after the Screen-edge group
        /// (which itself runs as one HIGH + one LOW pass, or one Stretch pass).
        const RAYCAST_CYCLING_SCREEN       = 1 << 9;
        const RAYCAST_CYCLING_SCREEN_EXTRA = 1 << 11;
        const RAYCAST_CYCLING_ALL          = 1 << 12;

        // Modifier: collapse HIGH/LOW pairs into a single bidirectional pass
        const RAYCAST_STRETCH              = 1 << 13;

        // Sort
        const SORT_AXIS_ORIGIN_X           = 1 << 14;
        const SORT_AXIS_ORIGIN_Y           = 1 << 15;
        /// Takes precedence over SORT_AXIS_ORIGIN_*. When set, results are
        /// sorted purely by squared Euclidean distance between window centers
        /// (origin's center → each candidate's center), closest first.
        const SORT_NEAREST                 = 1 << 16;
    }
}
