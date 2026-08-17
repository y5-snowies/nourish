bitflags::bitflags! {
    /// Layout action flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct LayoutFlags: u64 {
        const NONE                       = 0 << 0;
        const ALIGN                       = 1 << 0;
        const ALIGN_LEFT                  = 1 << 1;
        const ALIGN_RIGHT                 = 1 << 2;
        const ALIGN_TOP                   = 1 << 3;
        const ALIGN_BOTTOM                = 1 << 4;
        const ALIGN_CENTER_HORIZONTAL     = 1 << 5;
        const ALIGN_CENTER_VERTICAL       = 1 << 6;
        const ALIGN_STRETCH_LEFT          = 1 << 7;
        const ALIGN_STRETCH_RIGHT         = 1 << 8;
        const ALIGN_STRETCH_TOP           = 1 << 9;
        const ALIGN_STRETCH_BOTTOM        = 1 << 10;
        const ALIGN_CLOSE                 = 1 << 11;
        const DISTRIBUTE_HORIZONTALLY     = 1 << 12;
        const DISTRIBUTE_VERTICALLY       = 1 << 13;
        const DISTRIBUTE_TARGET_H_START         = 1 << 14;
        const DISTRIBUTE_TARGET_H_AVERAGE       = 1 << 15;
        const DISTRIBUTE_TARGET_H_MIN           = 1 << 16;
        const DISTRIBUTE_TARGET_H_MAX           = 1 << 17;
        const DISTRIBUTE_TARGET_H_AXIS          = 1 << 18;
        const DISTRIBUTE_TARGET_H_AXIS_BOUNDED  = 1 << 19;
        const DISTRIBUTE_TARGET_V_START         = 1 << 20;
        const DISTRIBUTE_TARGET_V_AVERAGE       = 1 << 21;
        const DISTRIBUTE_TARGET_V_MIN           = 1 << 22;
        const DISTRIBUTE_TARGET_V_MAX           = 1 << 23;
        const DISTRIBUTE_TARGET_V_AXIS          = 1 << 24;
        const DISTRIBUTE_TARGET_V_AXIS_BOUNDED  = 1 << 25;
    }
}
