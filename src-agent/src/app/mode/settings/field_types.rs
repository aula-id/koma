//! [`SettingField`] enum, its label helper, and the field list for the General page.

/// A single editable/toggleable field within a settings category.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SettingField {
    ApiKey,
    Model,
    Provider,
    // deprecated: superseded by Palette
    Theme,
    // deprecated: superseded by Palette
    Accent,
    /// Palette picker (Stage 4): arrow-cycles a named colour palette from
    /// [`crate::view::theme::PALETTES`]; replaces the Theme + Accent rows.
    Palette,
    Name,
    Workdir,
    /// Toggle: whether the project-awareness summary is generated/injected.
    AwarenessEnabled,
    /// Toggle: awareness model source — inherit the session model or use the
    /// dedicated awareness model/provider.
    AwarenessSource,
    /// Text: dedicated awareness model (ignored when the source is "inherit").
    AwarenessModel,
    /// Text: dedicated awareness provider (ignored when the source is "inherit").
    AwarenessProvider,
    /// Toggle: master switch for the safety harness ("Pass B").
    ClassifierEnabled,
    /// Text: model used for the safety classifier.
    ClassifierModel,
    /// Text: provider slug (strict-pinned) for the safety classifier.
    ClassifierProvider,
    /// Text: extra allowed folders (comma-separated) for the workspace check.
    AllowedFolders,
    /// Toggle: master kill-switch for the short-send token saver.
    ShortSendEnabled,
    /// Toggle: cache-warmth-adaptive summarization. On only for models with a
    /// sliding/refreshing prompt cache (e.g. Anthropic).
    SlidingCache,
    /// Toggle: whether bash/git_operator save filtered output logs to disk.
    BashSaving,
    /// Toggle: GUI Coding panel auto-save (debounced) for dirty editor tabs.
    CodingAutosave,
    /// Toggle: internet-access tier — `simple` (DDG in-process) vs `full`
    /// (scrapion Firefox subprocess, higher token usage).
    InternetMode,
}

impl SettingField {
    /// Human-readable label shown in the detail pane.
    pub fn label(self) -> &'static str {
        match self {
            SettingField::ApiKey            => "API key",
            SettingField::Model             => "Model",
            SettingField::Provider          => "Provider",
            SettingField::Theme             => "Theme",
            SettingField::Accent            => "Accent",
            SettingField::Palette           => "Theme",
            SettingField::Name              => "Session name",
            SettingField::Workdir           => "Workdir",
            SettingField::AwarenessEnabled  => "Awareness",
            SettingField::AwarenessSource   => "Model source",
            SettingField::AwarenessModel    => "Aware model",
            SettingField::AwarenessProvider => "Aware provider",
            SettingField::ClassifierEnabled  => "Harness",
            SettingField::ClassifierModel    => "Class. model",
            SettingField::ClassifierProvider => "Class. provider",
            SettingField::AllowedFolders     => "Allowed dirs",
            SettingField::ShortSendEnabled   => "Short-send",
            SettingField::SlidingCache       => "Sliding cache",
            SettingField::BashSaving         => "Bash shorts",
            SettingField::CodingAutosave     => "Coding autosave",
            SettingField::InternetMode       => "Internet mode",
        }
    }
}

/// The field list for the General page (session-level settings).
pub const GENERAL_FIELDS: &[SettingField] = &[
    SettingField::Name,
    SettingField::Workdir,
    SettingField::AwarenessEnabled,
    SettingField::AwarenessSource,
    SettingField::AwarenessModel,
    SettingField::AwarenessProvider,
    SettingField::ClassifierEnabled,
    SettingField::ClassifierModel,
    SettingField::ClassifierProvider,
    SettingField::AllowedFolders,
    SettingField::ShortSendEnabled,
    SettingField::SlidingCache,
    SettingField::BashSaving,
    SettingField::CodingAutosave,
    SettingField::InternetMode,
];
