//! The actool build whose output accord reproduces (`--xcode-version`).

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum XcodeVersion {
    /// Xcode 26.6.
    V26,
    /// Xcode 27.0.
    #[default]
    V27,
}

/// CARHEADER and EXTENDED_METADATA strings of one build.
pub struct VersionStrings {
    pub coreui: &'static str,
    pub xcode: &'static str,
    pub authoring_tool: &'static str,
}

impl XcodeVersion {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "26" => Some(Self::V26),
            "27" => Some(Self::V27),
            _ => None,
        }
    }

    /// Both are at least 975, below which CoreUI skips the `.icon` code path and every
    /// `imagesWithName:` lookup comes back empty.
    pub fn coreui_version(self) -> u32 {
        match self {
            Self::V26 => 975,
            Self::V27 => 1010,
        }
    }

    pub fn version_strings(self, platform: &str) -> VersionStrings {
        match (self, platform == "macosx") {
            (Self::V26, true) => VersionStrings {
                coreui: "@(#)PROGRAM:CoreUI  PROJECT:CoreUI-975 [LAR]",
                xcode: "Xcode 26.6 (17F113) via AssetCatalogAgent-AssetRuntime",
                authoring_tool: "@(#)PROGRAM:CoreThemeDefinition  PROJECT:CoreThemeDefinition-653.4 [LAR] [IIO-2784.5.4][LAR] AppleJPEG[1][32]  CMPhoto[1][174]]",
            },
            (Self::V26, false) => VersionStrings {
                coreui: "@(#)PROGRAM:CoreUI  PROJECT:CoreUI-975",
                xcode: "Xcode 26.6 (17F113) via AssetCatalogSimulatorAgent",
                authoring_tool: "@(#)PROGRAM:CoreThemeDefinition  PROJECT:CoreThemeDefinition-653.4  [IIO-2784.5.4]",
            },
            (Self::V27, _) => VersionStrings {
                coreui: "@(#)PROGRAM:CoreUI  PROJECT:CoreUI-1010 [LAR]",
                xcode: "Xcode 27.0 (27A266a) via AssetCatalogAgent-AssetRuntime",
                authoring_tool: "@(#)PROGRAM:CoreThemeDefinition  PROJECT:CoreThemeDefinition-664 [LAR] [IIO-2851][LAR] AppleJPEG[1][32]  CMPhoto[1][218]] [IR-120(hw)]",
            },
        }
    }

    /// TLV 1022 on IconStack and IconGroup renditions.
    pub fn writes_tlv_1022(self) -> bool {
        match self {
            Self::V26 => false,
            Self::V27 => true,
        }
    }

    /// The value of an IconGroup row in TLV 1020 and of a group entry in TLV 1022.
    pub fn group_value(self, blur_material: Option<f32>) -> f32 {
        match self {
            Self::V26 => 0.0,
            Self::V27 => blur_material.unwrap_or(0.0),
        }
    }

    /// Whether a gradient of the system background gray pair is named `system-light` or
    /// `system-dark` instead of numbered.
    pub fn names_system_gradients(self) -> bool {
        match self {
            Self::V26 => false,
            Self::V27 => true,
        }
    }

    /// Whether `--enable-icon-stack-fallback-generation NO` leaves out the pre-rendered icons.
    pub fn honours_fallback_no(self) -> bool {
        match self {
            Self::V26 => true,
            Self::V27 => false,
        }
    }
}
