//! Stateful translation of MinUI events into mode-aware editor intents.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
    time::{Duration, Instant},
};

use minui::KeybindAction;
use minui::prelude::input::{Event, KeyKind, KeyModifiers, KeyWithModifiers};
use redox_core::{DelimiterKind, TextObjectKind, TextObjectScope, TextObjectSpec, motion::Motion};

pub mod cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputMode {
    Normal,
    Insert,
    Command,
    Search,
    Finder,
    PinSelect,
    LspMarketplace,
    DiagnosticsList,
    CodeActions,
    SymbolInfo,
    Visual,
    VisualLine,
    VisualBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertKind {
    Insert,

    Append,

    InsertLineStart,

    AppendLineEnd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    Paste(String),

    Motion {
        motion: Motion,
        count: usize,
    },

    SetMode(InputMode),

    EnterInsert(InsertKind),
    OpenLineBelow,
    OpenLineAbove,
    JoinLineBelow,

    EnterCommand,
    EnterSearch,
    OpenExplorer,
    ToggleUndoTree,
    OpenFinder,
    ToggleDiagnosticsList,
    TriggerCodeActions,
    GotoDefinition,
    TriggerSymbolInfo,
    TriggerCompletion,
    CompletionMoveNext,
    CompletionMovePrev,
    CompletionAccept,
    CompletionCancel,
    SnippetNext,
    SurfaceOpenSelected,
    SurfaceGoParent,
    ViewportDownCenter,
    ViewportUpCenter,
    CenterCursorLine,
    ReplaySequence(String),
    RunCommand(String),
    SplitFocusLeft,
    SplitFocusDown,
    SplitFocusUp,
    SplitFocusRight,
    SplitHorizontal,
    SplitVertical,
    CloseSplit,
    Undo,
    Redo,
    ConfirmExplorerDelete,
    YankSelectionPrivate,
    DeleteSelectionPrivate,
    ChangeSelectionPrivate,
    DeleteSelectionNoYank,
    OperateTarget {
        operator: TextObjectOperator,
        target: OperatorTarget,
    },
    DeleteCurrentLinePrivate {
        count: usize,
    },
    YankCurrentLinePrivate {
        count: usize,
    },
    ChangeCurrentLinePrivate {
        count: usize,
    },
    YankSelectionSystem,
    PasteSystemClipboard,
    PasteSystemClipboardText(String),
    PastePrivateRegister,
    PastePrivateRegisterBefore,
    DeleteCharNoYank,
    ToggleCase {
        count: usize,
    },
    ReplaceChar(char),
    MoveVisualSelectionUp {
        count: usize,
    },
    MoveVisualSelectionDown {
        count: usize,
    },
    IndentVisualSelection {
        count: usize,
    },
    OutdentVisualSelection {
        count: usize,
    },

    CommandChar(char),
    CommandBackspace,
    CommandMoveLeft,
    CommandMoveRight,
    CommandHistoryPrev,
    CommandHistoryNext,
    CommandEnter,
    CommandCancel,

    SearchChar(char),
    SearchBackspace,
    SearchMoveLeft,
    SearchMoveRight,
    SearchEnter,
    SearchCancel,
    FinderChar(char),
    FinderBackspace,
    FinderMoveLeft,
    FinderMoveRight,
    FinderMoveNext,
    FinderMovePrev,
    FinderEnter,
    FinderCancel,
    FinderBeginPin,
    PinSelectorMoveNext,
    PinSelectorMovePrev,
    PinSelectorOpenSelected,
    PinSelectorAssign,
    PinSelectorReorderUp,
    PinSelectorReorderDown,
    PinSelectorDeleteSelected,
    PinSelectorCancel,
    LspMarketplaceMoveNext,
    LspMarketplaceMovePrev,
    LspMarketplaceInstallSelected,
    LspMarketplaceUninstallSelected,
    LspMarketplaceCancel,
    DiagnosticsListMoveNext,
    DiagnosticsListMovePrev,
    DiagnosticsListOpenSelected,
    DiagnosticsListCancel,
    CodeActionsMoveNext,
    CodeActionsMovePrev,
    CodeActionsApplySelected,
    CodeActionsCancel,
    SymbolInfoMoveNext,
    SymbolInfoMovePrev,
    SymbolInfoCancel,
    AssignPinSlot {
        slot: usize,
    },
    OpenPinnedSlot {
        slot: usize,
    },
    QuickPinCurrentFile,

    RepeatSearch {
        forward: bool,
    },
    ClearSearch,

    InsertChar(char),
    Backspace,
    Enter,

    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorTarget {
    Motion { motion: Motion, count: usize },
    TextObject(TextObjectSpec),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectOperator {
    Delete,
    Change,
    Select,
    Yank,
}

pub const DEFAULT_WHICH_KEY_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhichKeyEntry {
    pub key: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhichKeyPopup {
    pub prefix: String,
    pub entries: Vec<WhichKeyEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefixFallback {
    Consume,
    RetryCurrent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SequenceAction {
    OpenExplorer,
    ToggleUndoTree,
    OpenFinder,
    ToggleDiagnosticsList,
    TriggerCodeActions,
    GotoDefinition,
    YankSelectionSystem,
    PasteSystemClipboard,
    FileStart,
    CenterCursorLine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SequenceBinding {
    sequence: &'static str,
    fallback: PrefixFallback,
    action: Option<SequenceAction>,
}

const COMMON_SEQUENCE_BINDINGS: &[SequenceBinding] = &[
    SequenceBinding {
        sequence: " ",
        fallback: PrefixFallback::Consume,
        action: None,
    },
    SequenceBinding {
        sequence: " e",
        fallback: PrefixFallback::Consume,
        action: Some(SequenceAction::OpenExplorer),
    },
    SequenceBinding {
        sequence: " u",
        fallback: PrefixFallback::Consume,
        action: Some(SequenceAction::ToggleUndoTree),
    },
    SequenceBinding {
        sequence: " c",
        fallback: PrefixFallback::Consume,
        action: None,
    },
    SequenceBinding {
        sequence: " ca",
        fallback: PrefixFallback::Consume,
        action: Some(SequenceAction::TriggerCodeActions),
    },
    SequenceBinding {
        sequence: " x",
        fallback: PrefixFallback::Consume,
        action: Some(SequenceAction::ToggleDiagnosticsList),
    },
    SequenceBinding {
        sequence: "  ",
        fallback: PrefixFallback::Consume,
        action: Some(SequenceAction::OpenFinder),
    },
    SequenceBinding {
        sequence: "g",
        fallback: PrefixFallback::RetryCurrent,
        action: None,
    },
    SequenceBinding {
        sequence: "gg",
        fallback: PrefixFallback::RetryCurrent,
        action: Some(SequenceAction::FileStart),
    },
    SequenceBinding {
        sequence: "gd",
        fallback: PrefixFallback::RetryCurrent,
        action: Some(SequenceAction::GotoDefinition),
    },
];

const NORMAL_SEQUENCE_BINDINGS: &[SequenceBinding] = &[
    SequenceBinding {
        sequence: " p",
        fallback: PrefixFallback::Consume,
        action: Some(SequenceAction::PasteSystemClipboard),
    },
    SequenceBinding {
        sequence: "z",
        fallback: PrefixFallback::RetryCurrent,
        action: None,
    },
    SequenceBinding {
        sequence: "zz",
        fallback: PrefixFallback::RetryCurrent,
        action: Some(SequenceAction::CenterCursorLine),
    },
];

const VISUAL_SEQUENCE_BINDINGS: &[SequenceBinding] = &[SequenceBinding {
    sequence: " y",
    fallback: PrefixFallback::Consume,
    action: Some(SequenceAction::YankSelectionSystem),
}];

/// State machine for multi-key sequences and counts.
#[derive(Debug, Clone)]
pub struct InputState {
    pending_sequence: String,
    pending_count: Option<usize>,
    pending_operator: Option<PendingOperator>,
    pending_search_motion: Option<PendingSearchMotion>,
    pending_replace: bool,
    which_key_started_at: Option<Instant>,
    leader: char,
    custom_bindings: Vec<CustomBinding>,
}

#[derive(Debug, Clone)]
struct CustomBinding {
    mode: InputMode,
    key: CustomKey,
    action: InputAction,
    description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredBinding {
    pub mode: String,
    pub keys: String,
    pub target: ConfiguredBindingTarget,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfiguredBindingTarget {
    Sequence(String),
    Command(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CustomKey {
    Sequence(String),
    Special(String),
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            pending_sequence: String::new(),
            pending_count: None,
            pending_operator: None,
            pending_search_motion: None,
            pending_replace: false,
            which_key_started_at: None,
            leader: ' ',
            custom_bindings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingOperator {
    operator: TextObjectOperator,
    count: usize,
    count_explicit: bool,
    scope: Option<TextObjectScope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingSearchMotion {
    operator: Option<PendingOperator>,
    count: usize,
    count_explicit: bool,
    kind: SearchMotionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchMotionKind {
    Find,
    Till,
    FindBefore,
    TillBefore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingInput {
    sequence: String,
    count: Option<usize>,
    operator: Option<PendingOperator>,
    search_motion: Option<PendingSearchMotion>,
    replace: bool,
}

impl PendingInput {
    fn typed_prefix(&self) -> String {
        if let Some(search) = self.search_motion {
            return search_motion_prefix(search);
        }
        if self.replace {
            return "r".to_string();
        }
        if let Some(operator) = self.operator {
            return operator_prefix(operator, &self.sequence);
        }
        if !self.sequence.is_empty() {
            return self.sequence.clone();
        }
        self.count
            .map(|count| count.to_string())
            .unwrap_or_default()
    }
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn configure(
        &mut self,
        leader: char,
        bindings: &BTreeMap<String, BTreeMap<String, String>>,
    ) -> anyhow::Result<()> {
        let mut custom_bindings = Vec::new();
        let mut configured_keys = HashSet::new();
        for (mode_name, entries) in bindings {
            let mode = input_mode_from_name(mode_name)
                .ok_or_else(|| anyhow::anyhow!("unknown keybinding mode {mode_name:?}"))?;
            for (action, keys) in entries {
                let (action, description) = configured_action(action)?;
                if !keys.contains("<leader>") && keys.starts_with('<') && keys.ends_with('>') {
                    let key = CustomKey::Special(normalize_special_key(keys)?);
                    ensure_unique_binding(&mut configured_keys, mode, &key)?;
                    custom_bindings.push(CustomBinding {
                        mode,
                        key,
                        action,
                        description: description.to_string(),
                    });
                    continue;
                }
                let sequence = keys.replace("<leader>", &leader.to_string());
                if sequence.is_empty() {
                    anyhow::bail!("keybinding sequence cannot be empty");
                }
                if sequence.contains('<') || sequence.contains('>') {
                    anyhow::bail!(
                        "keybinding {keys:?} mixes a character sequence with a modified key"
                    );
                }
                if !modal_sequence_mode(mode) && sequence.chars().count() != 1 {
                    anyhow::bail!(
                        "keybinding mode {mode_name:?} supports only one-character sequences"
                    );
                }
                let key = CustomKey::Sequence(sequence);
                ensure_unique_binding(&mut configured_keys, mode, &key)?;
                custom_bindings.push(CustomBinding {
                    mode,
                    key,
                    action,
                    description: description.to_string(),
                });
            }
        }
        reject_ambiguous_sequence_prefixes(&custom_bindings)?;
        self.leader = leader;
        self.custom_bindings = custom_bindings;
        Ok(())
    }

    pub fn configure_custom_bindings(
        &mut self,
        bindings: &[ConfiguredBinding],
    ) -> anyhow::Result<()> {
        let mut next_bindings = self.custom_bindings.clone();
        let mut configured_keys = next_bindings
            .iter()
            .map(|binding| (binding.mode, binding.key.clone()))
            .collect::<HashSet<_>>();

        for binding in bindings {
            if binding.keys.is_empty() {
                continue;
            }
            let mode = input_mode_from_name(&binding.mode)
                .ok_or_else(|| anyhow::anyhow!("unknown custom binding mode {:?}", binding.mode))?;
            if !modal_sequence_mode(mode) {
                anyhow::bail!(
                    "custom binding mode {:?} must be a modal mode",
                    binding.mode
                );
            }
            let description = binding.description.trim();
            if description.is_empty() {
                anyhow::bail!("custom binding description cannot be empty");
            }

            let keys = binding.keys.replace("<leader>", &self.leader.to_string());
            if keys.contains('<') || keys.contains('>') {
                anyhow::bail!(
                    "custom binding {:?} must be a character sequence",
                    binding.keys
                );
            }
            let key = CustomKey::Sequence(keys);
            ensure_unique_binding(&mut configured_keys, mode, &key)?;

            let action = match &binding.target {
                ConfiguredBindingTarget::Sequence(sequence) => {
                    let sequence = sequence.replace("<leader>", &self.leader.to_string());
                    if sequence.is_empty() {
                        anyhow::bail!("custom binding sequence cannot be empty");
                    }
                    if sequence.contains('<') || sequence.contains('>') {
                        anyhow::bail!(
                            "custom binding target {:?} must be a character sequence",
                            sequence
                        );
                    }
                    InputAction::ReplaySequence(sequence)
                }
                ConfiguredBindingTarget::Command(command) => {
                    let command = command.trim().trim_start_matches(':').trim();
                    if command.is_empty() {
                        anyhow::bail!("custom binding command cannot be empty");
                    }
                    InputAction::RunCommand(command.to_string())
                }
            };
            next_bindings.push(CustomBinding {
                mode,
                key,
                action,
                description: description.to_string(),
            });
        }

        reject_ambiguous_sequence_prefixes(&next_bindings)?;
        let mut validator = self.clone();
        validator.custom_bindings = next_bindings.clone();
        for binding in &next_bindings {
            if let InputAction::ReplaySequence(sequence) = &binding.action {
                validator.expand_replay_sequence(binding.mode, sequence)?;
            }
        }
        self.custom_bindings = next_bindings;
        Ok(())
    }

    pub fn expand_replay_sequence(
        &self,
        mode: InputMode,
        sequence: &str,
    ) -> anyhow::Result<Vec<InputAction>> {
        self.expand_replay_sequence_inner(mode, sequence, &mut Vec::new())
    }

    fn expand_replay_sequence_inner(
        &self,
        mode: InputMode,
        sequence: &str,
        stack: &mut Vec<String>,
    ) -> anyhow::Result<Vec<InputAction>> {
        let marker = format!("{mode:?}:{sequence}");
        if stack.contains(&marker) {
            anyhow::bail!("custom binding sequence contains a cycle at {sequence:?}");
        }
        stack.push(marker);

        let mut parser = self.clone();
        parser.reset_prefixes();
        let mut actions = Vec::new();
        for key in sequence.chars() {
            let action = map_event_with_state(&mut parser, mode, &Event::Character(key));
            match action {
                InputAction::None => {}
                InputAction::ReplaySequence(nested) => {
                    actions.extend(self.expand_replay_sequence_inner(mode, &nested, stack)?);
                }
                action if replayable_motion_action(&action) => actions.push(action),
                action => anyhow::bail!(
                    "custom binding sequence {sequence:?} resolves to non-motion action {action:?}"
                ),
            }
        }
        if parser.pending_input().is_some() {
            anyhow::bail!("custom binding sequence {sequence:?} is incomplete");
        }
        let _ = stack.pop();
        if actions.is_empty() {
            anyhow::bail!("custom binding sequence {sequence:?} does not perform a motion");
        }
        Ok(actions)
    }

    pub fn special_keys(&self) -> impl Iterator<Item = &str> {
        self.custom_bindings
            .iter()
            .filter_map(|binding| match &binding.key {
                CustomKey::Special(key) => Some(key.as_str()),
                CustomKey::Sequence(_) => None,
            })
    }

    fn leader(&self) -> char {
        self.leader
    }

    pub fn reset_prefixes(&mut self) {
        self.pending_sequence.clear();
        self.pending_count = None;
        self.pending_operator = None;
        self.pending_search_motion = None;
        self.pending_replace = false;
        self.which_key_started_at = None;
    }

    fn pending_input(&self) -> Option<PendingInput> {
        let pending = PendingInput {
            sequence: self.pending_sequence.clone(),
            count: self.pending_count,
            operator: self.pending_operator,
            search_motion: self.pending_search_motion,
            replace: self.pending_replace,
        };
        (!pending.sequence.is_empty()
            || pending.count.is_some()
            || pending.operator.is_some()
            || pending.search_motion.is_some()
            || pending.replace)
            .then_some(pending)
    }

    fn update_which_key_timer(&mut self, previous: Option<PendingInput>) {
        let current = self.pending_input();
        if current.is_none() {
            self.which_key_started_at = None;
        } else if self.which_key_started_at.is_none()
            || !previous
                .as_ref()
                .zip(current.as_ref())
                .is_some_and(|(previous, current)| {
                    current.typed_prefix().starts_with(&previous.typed_prefix())
                })
        {
            self.which_key_started_at = Some(Instant::now());
        }
    }

    fn push_count_digit(&mut self, d: u8) {
        debug_assert!(d <= 9);
        let current = self.pending_count.unwrap_or(0);
        let next = current.saturating_mul(10).saturating_add(d as usize);
        self.pending_count = Some(next);
    }

    fn take_count_or_1(&mut self) -> usize {
        self.take_count().0
    }

    fn take_count(&mut self) -> (usize, bool) {
        match self.pending_count.take() {
            Some(0) | None => (1, false),
            Some(n) => (n, true),
        }
    }

    fn push_sequence_char(&mut self, c: char) {
        self.pending_sequence.push(c);
    }

    fn clear_sequence(&mut self) {
        self.pending_sequence.clear();
    }

    fn begin_replace(&mut self) {
        self.pending_sequence.clear();
        self.pending_count = None;
        self.pending_operator = None;
        self.pending_search_motion = None;
        self.pending_replace = true;
    }

    fn begin_search_motion(
        &mut self,
        operator: Option<PendingOperator>,
        kind: SearchMotionKind,
        count: usize,
        count_explicit: bool,
    ) {
        self.pending_sequence.clear();
        self.pending_count = None;
        self.pending_operator = None;
        self.pending_replace = false;
        self.pending_search_motion = Some(PendingSearchMotion {
            operator,
            count: count.max(1),
            count_explicit,
            kind,
        });
    }

    fn begin_operator(&mut self, operator: TextObjectOperator, scope: Option<TextObjectScope>) {
        let (count, count_explicit) = self.take_count();
        self.pending_operator = Some(PendingOperator {
            operator,
            count,
            count_explicit,
            scope,
        });
    }

    fn backspace_pending(&mut self) -> bool {
        if self.pending_input().is_none() {
            return false;
        }

        if self.pending_sequence.pop().is_some() {
            self.clear_which_key_timer_if_complete();
            return true;
        }
        if let Some(search) = self.pending_search_motion.take() {
            if let Some(operator) = search.operator {
                self.pending_operator = Some(operator);
            } else if search.count_explicit {
                self.pending_count = Some(search.count);
            }
            self.clear_which_key_timer_if_complete();
            return true;
        }
        if self.pending_replace {
            self.pending_replace = false;
            self.clear_which_key_timer_if_complete();
            return true;
        }
        if self.pending_operator.is_some_and(|operator| {
            operator.operator == TextObjectOperator::Select && operator.scope.is_some()
        }) {
            let operator = self
                .pending_operator
                .take()
                .expect("checked pending operator");
            if operator.count_explicit {
                self.pending_count = Some(operator.count);
            }
            self.clear_which_key_timer_if_complete();
            return true;
        }
        if let Some(operator) = self.pending_operator.as_mut()
            && operator.scope.take().is_some()
        {
            return true;
        }
        if let Some(operator) = self.pending_operator.take() {
            if operator.count_explicit {
                self.pending_count = Some(operator.count);
            }
            self.clear_which_key_timer_if_complete();
            return true;
        }
        if let Some(count) = self.pending_count {
            self.pending_count = (count >= 10).then_some(count / 10);
            self.clear_which_key_timer_if_complete();
            return true;
        }
        false
    }

    fn clear_which_key_timer_if_complete(&mut self) {
        if self.pending_input().is_none() {
            self.which_key_started_at = None;
        }
    }

    pub fn which_key_popup(
        &self,
        mode: InputMode,
        now: Instant,
        delay: Duration,
    ) -> Option<WhichKeyPopup> {
        let started_at = self.which_key_started_at?;
        if now.saturating_duration_since(started_at) < delay {
            return None;
        }

        let (prefix, entries) = self.which_key_contents(mode)?;
        (!entries.is_empty()).then_some(WhichKeyPopup { prefix, entries })
    }

    fn which_key_contents(&self, mode: InputMode) -> Option<(String, Vec<WhichKeyEntry>)> {
        if let Some(search) = self.pending_search_motion {
            let description = match search.kind {
                SearchMotionKind::Find => "Find character forwards",
                SearchMotionKind::Till => "Until character forwards",
                SearchMotionKind::FindBefore => "Find character backwards",
                SearchMotionKind::TillBefore => "Until character backwards",
            };
            return Some((
                search_motion_prefix(search),
                vec![which_key_entry("<char>", description)],
            ));
        }

        if self.pending_replace {
            return Some((
                "r".to_string(),
                vec![which_key_entry("<char>", "Replace character")],
            ));
        }

        if let Some(operator) = self.pending_operator {
            let prefix = operator_prefix(operator, &self.pending_sequence);
            let entries = if operator.scope.is_some() {
                text_object_entries()
            } else if self.pending_sequence.is_empty() {
                operator_entries(operator.operator)
            } else {
                vec![which_key_entry("g", "Start of file")]
            };
            return Some((prefix, entries));
        }

        if !self.pending_sequence.is_empty() {
            return Some((
                display_sequence(&self.pending_sequence, self.leader),
                sequence_entries(self, mode, &self.pending_sequence),
            ));
        }

        self.pending_count
            .map(|count| (count.to_string(), count_entries(mode)))
    }
}

fn which_key_entry(key: impl Into<String>, description: impl Into<String>) -> WhichKeyEntry {
    WhichKeyEntry {
        key: key.into(),
        description: description.into(),
    }
}

fn replayable_motion_action(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::Motion { .. }
            | InputAction::ViewportDownCenter
            | InputAction::ViewportUpCenter
            | InputAction::CenterCursorLine
    )
}

fn sequence_entries(state: &InputState, mode: InputMode, prefix: &str) -> Vec<WhichKeyEntry> {
    #[derive(Default)]
    struct Node {
        exact: Option<String>,
        group: Option<String>,
        has_children: bool,
        exact_precedes_children: bool,
    }

    fn add_mapping(
        nodes: &mut BTreeMap<String, Node>,
        remainder: &str,
        description: &str,
        is_group: bool,
        exact_precedes_children: bool,
    ) {
        let Some(next) = remainder.chars().next() else {
            return;
        };
        let next_len = next.len_utf8();
        let node = nodes.entry(next.to_string()).or_default();
        if remainder.len() > next_len || is_group {
            node.has_children = true;
            if is_group {
                node.group = Some(description.to_string());
            }
        } else {
            node.exact = Some(description.to_string());
            node.exact_precedes_children = exact_precedes_children;
        }
    }

    let mut nodes = BTreeMap::<String, Node>::new();
    for binding in sequence_bindings_for_mode(mode) {
        let sequence = builtin_sequence(binding.sequence, state.leader());
        let Some(remainder) = sequence.strip_prefix(prefix) else {
            continue;
        };
        if remainder.is_empty() {
            continue;
        }
        add_mapping(
            &mut nodes,
            remainder,
            sequence_action_description(binding),
            binding.action.is_none(),
            false,
        );
    }
    for binding in &state.custom_bindings {
        let CustomKey::Sequence(sequence) = &binding.key else {
            continue;
        };
        if binding.mode != mode {
            continue;
        }
        let Some(remainder) = sequence.strip_prefix(prefix) else {
            continue;
        };
        if remainder.is_empty() {
            continue;
        }
        add_mapping(&mut nodes, remainder, &binding.description, false, true);
    }
    nodes
        .into_iter()
        .filter_map(|(key, node)| {
            let description = if node.exact_precedes_children {
                node.exact?
            } else if node.has_children {
                node.group.unwrap_or_else(|| "+prefix".to_string())
            } else {
                node.exact?
            };
            Some(which_key_entry(
                display_sequence(&key, state.leader()),
                description,
            ))
        })
        .collect()
}

fn sequence_action_description(binding: &SequenceBinding) -> &'static str {
    match binding.action {
        Some(SequenceAction::OpenExplorer) => "Open explorer",
        Some(SequenceAction::ToggleUndoTree) => "Toggle undo tree",
        Some(SequenceAction::OpenFinder) => "Find files",
        Some(SequenceAction::ToggleDiagnosticsList) => "Toggle diagnostics",
        Some(SequenceAction::TriggerCodeActions) => "Code actions",
        Some(SequenceAction::GotoDefinition) => "Go to definition",
        Some(SequenceAction::YankSelectionSystem) => "Yank to system clipboard",
        Some(SequenceAction::PasteSystemClipboard) => "Paste system clipboard",
        Some(SequenceAction::FileStart) => "Start of file",
        Some(SequenceAction::CenterCursorLine) => "Centre cursor line",
        None if binding.sequence.ends_with('c') => "+code",
        None => "+prefix",
    }
}

fn operator_entries(operator: TextObjectOperator) -> Vec<WhichKeyEntry> {
    let repeat_description = match operator {
        TextObjectOperator::Delete => "Delete current line",
        TextObjectOperator::Change => "Change current line",
        TextObjectOperator::Yank => "Yank current line",
        TextObjectOperator::Select => "Select current line",
    };
    let mut entries = vec![which_key_entry(operator_key(operator), repeat_description)];
    entries.extend([
        which_key_entry("0", "Start of line"),
        which_key_entry("_", "First non-blank"),
        which_key_entry("$", "End of line"),
        which_key_entry("h", "Move left"),
        which_key_entry("j", "Move down"),
        which_key_entry("k", "Move up"),
        which_key_entry("l", "Move right"),
        which_key_entry("w", "Next word"),
        which_key_entry("b", "Previous word"),
        which_key_entry("e", "End of word"),
        which_key_entry("G", "End of file"),
        which_key_entry("g", "+go to"),
        which_key_entry("%", "Matching delimiter"),
        which_key_entry("f", "Find character forwards"),
        which_key_entry("t", "Until character forwards"),
        which_key_entry("F", "Find character backwards"),
        which_key_entry("T", "Until character backwards"),
        which_key_entry("i", "+inside text object"),
        which_key_entry("a", "+around text object"),
    ]);
    entries
}

fn text_object_entries() -> Vec<WhichKeyEntry> {
    [
        ("w", "Word"),
        ("W", "Whitespace-delimited word"),
        ("p", "Paragraph"),
        ("(", "Parentheses"),
        (")", "Parentheses"),
        ("b", "Parentheses"),
        ("[", "Brackets"),
        ("]", "Brackets"),
        ("{", "Braces"),
        ("}", "Braces"),
        ("B", "Braces"),
        ("'", "Single quotes"),
        ("\"", "Double quotes"),
        ("`", "Backticks"),
    ]
    .into_iter()
    .map(|(key, description)| which_key_entry(key, description))
    .collect()
}

fn count_entries(mode: InputMode) -> Vec<WhichKeyEntry> {
    let mut entries = [
        ("h", "Move left"),
        ("j", "Move down"),
        ("k", "Move up"),
        ("l", "Move right"),
        ("w", "Next word"),
        ("b", "Previous word"),
        ("e", "End of word"),
        ("_", "First non-blank"),
        ("$", "End of line"),
        ("G", "End of file"),
        ("g", "+go to"),
        ("%", "Matching delimiter"),
        ("f", "Find character forwards"),
        ("t", "Until character forwards"),
        ("F", "Find character backwards"),
        ("T", "Until character backwards"),
    ]
    .into_iter()
    .map(|(key, description)| which_key_entry(key, description))
    .collect::<Vec<_>>();
    if mode == InputMode::Normal {
        entries.extend([
            which_key_entry("d", "+delete"),
            which_key_entry("c", "+change"),
            which_key_entry("y", "+yank"),
        ]);
    }
    entries
}

fn operator_prefix(operator: PendingOperator, sequence: &str) -> String {
    let mut prefix = count_prefix(operator.count, operator.count_explicit);
    if operator.operator != TextObjectOperator::Select {
        prefix.push(operator_key(operator.operator));
    }
    if let Some(scope) = operator.scope {
        prefix.push(match scope {
            TextObjectScope::Inner => 'i',
            TextObjectScope::Around => 'a',
        });
    }
    prefix.push_str(sequence);
    prefix
}

fn search_motion_prefix(search: PendingSearchMotion) -> String {
    let mut prefix = search
        .operator
        .map(|operator| operator_prefix(operator, ""))
        .unwrap_or_else(|| count_prefix(search.count, search.count_explicit));
    prefix.push(match search.kind {
        SearchMotionKind::Find => 'f',
        SearchMotionKind::Till => 't',
        SearchMotionKind::FindBefore => 'F',
        SearchMotionKind::TillBefore => 'T',
    });
    prefix
}

fn count_prefix(count: usize, explicit: bool) -> String {
    explicit.then(|| count.to_string()).unwrap_or_default()
}

fn operator_key(operator: TextObjectOperator) -> char {
    match operator {
        TextObjectOperator::Delete => 'd',
        TextObjectOperator::Change => 'c',
        TextObjectOperator::Yank => 'y',
        TextObjectOperator::Select => 'v',
    }
}

fn display_sequence(sequence: &str, leader: char) -> String {
    if sequence.starts_with(leader) {
        sequence.replacen(leader, "<leader>", 1)
    } else {
        sequence.to_string()
    }
}

/// Map a MinUI `Event` to an `InputAction`, updating key-prefix state.
#[cfg_attr(not(test), allow(dead_code))]
pub fn map_event_with_state(state: &mut InputState, mode: InputMode, event: &Event) -> InputAction {
    map_event_with_context(state, mode, false, event)
}

pub fn map_event_with_context(
    state: &mut InputState,
    mode: InputMode,
    confirm_explorer_delete: bool,
    event: &Event,
) -> InputAction {
    if modal_sequence_mode(mode) && state.pending_input().is_some() {
        if pending_escape_event(event) {
            state.reset_prefixes();
            return InputAction::None;
        }
        if pending_backspace_event(event) && state.backspace_pending() {
            return InputAction::None;
        }
    }

    let previous = state.pending_input();
    let action = map_event_with_context_inner(state, mode, confirm_explorer_delete, event);
    state.update_which_key_timer(previous);
    action
}

fn pending_escape_event(event: &Event) -> bool {
    matches!(event, Event::Escape)
        || matches!(event, Event::KeyWithModifiers(KeyWithModifiers {
            key: KeyKind::Escape,
            mods,
        }) if unmodified(*mods))
}

fn pending_backspace_event(event: &Event) -> bool {
    matches!(event, Event::Backspace)
        || matches!(event, Event::KeyWithModifiers(KeyWithModifiers {
            key: KeyKind::Backspace,
            mods,
        }) if unmodified(*mods))
}

fn map_event_with_context_inner(
    state: &mut InputState,
    mode: InputMode,
    confirm_explorer_delete: bool,
    event: &Event,
) -> InputAction {
    let custom_character = match event {
        Event::Character(c) => Some(*c),
        Event::KeyWithModifiers(KeyWithModifiers {
            key: KeyKind::Char(c),
            mods,
        }) if text_mods(*mods) => Some(replacement_char_from_key(*c, *mods)),
        _ => None,
    };
    if let Some(c) = custom_character
        && !matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        )
        && let Some(action) = custom_sequence_action(state, mode, &c.to_string())
    {
        return finish_custom_action(state, action);
    }
    match event {
        Event::Escape => {
            state.reset_prefixes();
            match mode {
                InputMode::Insert => InputAction::CompletionCancel,
                InputMode::Command => InputAction::CommandCancel,
                InputMode::Search => InputAction::SearchCancel,
                InputMode::Finder => InputAction::FinderCancel,
                InputMode::PinSelect => InputAction::PinSelectorCancel,
                InputMode::LspMarketplace => InputAction::LspMarketplaceCancel,
                InputMode::DiagnosticsList => InputAction::DiagnosticsListCancel,
                InputMode::CodeActions => InputAction::CodeActionsCancel,
                InputMode::SymbolInfo => InputAction::SymbolInfoCancel,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {
                    InputAction::SetMode(InputMode::Normal)
                }
                InputMode::Normal => InputAction::ClearSearch,
            }
        }

        Event::Backspace => {
            state.reset_prefixes();
            match mode {
                InputMode::Insert => InputAction::Backspace,
                InputMode::Command => InputAction::CommandBackspace,
                InputMode::Search => InputAction::SearchBackspace,
                InputMode::Finder => InputAction::FinderBackspace,
                InputMode::PinSelect => InputAction::None,
                InputMode::LspMarketplace => InputAction::None,
                InputMode::DiagnosticsList => InputAction::None,
                InputMode::CodeActions => InputAction::None,
                InputMode::SymbolInfo => InputAction::None,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {
                    InputAction::None
                }
                InputMode::Normal => InputAction::None,
            }
        }

        Event::Enter => {
            state.reset_prefixes();
            match mode {
                InputMode::Insert => InputAction::Enter,
                InputMode::Command => InputAction::CommandEnter,
                InputMode::Search => InputAction::SearchEnter,
                InputMode::Finder => InputAction::FinderEnter,
                InputMode::PinSelect => InputAction::PinSelectorOpenSelected,
                InputMode::LspMarketplace => InputAction::None,
                InputMode::DiagnosticsList => InputAction::DiagnosticsListOpenSelected,
                InputMode::CodeActions => InputAction::CodeActionsApplySelected,
                InputMode::SymbolInfo => InputAction::None,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {
                    InputAction::None
                }
                InputMode::Normal => InputAction::SurfaceOpenSelected,
            }
        }

        Event::Keybind(KeybindAction::Custom(action)) if action == "trigger-completion" => {
            state.reset_prefixes();
            if mode == InputMode::Insert {
                InputAction::TriggerCompletion
            } else {
                InputAction::None
            }
        }

        Event::Keybind(KeybindAction::Custom(action)) if action == "trigger-symbol-info" => {
            state.reset_prefixes();
            if matches!(mode, InputMode::Insert | InputMode::Normal) {
                InputAction::TriggerSymbolInfo
            } else {
                InputAction::None
            }
        }

        Event::Keybind(KeybindAction::Custom(action)) if action.starts_with("redox-key:") => {
            let key = action.trim_start_matches("redox-key:");
            if let Some(action) = custom_special_action(state, mode, key) {
                finish_custom_action(state, action)
            } else {
                state.reset_prefixes();
                InputAction::None
            }
        }

        Event::KeyWithModifiers(k) => map_key_with_state(state, mode, confirm_explorer_delete, *k),

        Event::Character(c) => match mode {
            InputMode::Insert if *c == '\0' => {
                state.reset_prefixes();
                InputAction::None
            }
            InputMode::Insert => InputAction::InsertChar(*c),
            InputMode::Command => InputAction::CommandChar(*c),
            InputMode::Search => InputAction::SearchChar(*c),
            InputMode::Finder => InputAction::FinderChar(*c),
            InputMode::PinSelect => {
                state.reset_prefixes();
                match c {
                    'j' => InputAction::PinSelectorMoveNext,
                    'k' => InputAction::PinSelectorMovePrev,
                    'p' => InputAction::PinSelectorAssign,
                    'd' => InputAction::PinSelectorDeleteSelected,
                    _ => InputAction::None,
                }
            }
            InputMode::LspMarketplace => {
                state.reset_prefixes();
                match c {
                    'j' => InputAction::LspMarketplaceMoveNext,
                    'k' => InputAction::LspMarketplaceMovePrev,
                    'i' => InputAction::LspMarketplaceInstallSelected,
                    'u' => InputAction::LspMarketplaceUninstallSelected,
                    _ => InputAction::None,
                }
            }
            InputMode::DiagnosticsList => {
                state.reset_prefixes();
                match c {
                    'j' => InputAction::DiagnosticsListMoveNext,
                    'k' => InputAction::DiagnosticsListMovePrev,
                    'a' => InputAction::TriggerCodeActions,
                    _ => InputAction::None,
                }
            }
            InputMode::CodeActions => {
                state.reset_prefixes();
                match c {
                    'j' => InputAction::CodeActionsMoveNext,
                    'k' => InputAction::CodeActionsMovePrev,
                    _ => InputAction::None,
                }
            }
            InputMode::SymbolInfo => {
                state.reset_prefixes();
                match c {
                    'j' => InputAction::SymbolInfoMoveNext,
                    'k' => InputAction::SymbolInfoMovePrev,
                    _ => InputAction::None,
                }
            }
            InputMode::Normal
            | InputMode::Visual
            | InputMode::VisualLine
            | InputMode::VisualBlock => modal_char_action(state, mode, confirm_explorer_delete, *c),
        },

        _ => InputAction::None,
    }
}

fn modal_char_action(
    state: &mut InputState,
    mode: InputMode,
    confirm_explorer_delete: bool,
    c: char,
) -> InputAction {
    if state.pending_replace {
        state.reset_prefixes();
        return InputAction::ReplaceChar(c);
    }

    if let Some(pending_search_motion) = state.pending_search_motion {
        return resolve_pending_search_motion(state, c, pending_search_motion);
    }

    if let Some(operator) = state.pending_operator {
        if let Some(action) = resolve_pending_operator(state, c, operator) {
            return action;
        }
    }

    if !state.pending_sequence.is_empty() {
        if let Some(action) = resolve_pending_sequence(state, mode, c) {
            return action;
        }
    }

    if custom_sequence_starts(state, mode, c) {
        let candidate = c.to_string();
        if let Some(action) = custom_sequence_action(state, mode, &candidate) {
            return finish_custom_action(state, action);
        }
        state.push_sequence_char(c);
        return InputAction::None;
    }

    if let Some(d) = c.to_digit(10) {
        if d == 0 && state.pending_count.is_none() {
            state.reset_prefixes();
            return InputAction::Motion {
                motion: Motion::LineStart,
                count: 1,
            };
        }
        state.push_count_digit(d as u8);
        return InputAction::None;
    }

    match c {
        'v' => {
            state.reset_prefixes();
            match mode {
                InputMode::Normal => InputAction::SetMode(InputMode::Visual),
                InputMode::Visual => InputAction::SetMode(InputMode::Normal),
                InputMode::VisualLine => InputAction::SetMode(InputMode::Visual),
                InputMode::VisualBlock => InputAction::SetMode(InputMode::Visual),
                InputMode::Insert
                | InputMode::Command
                | InputMode::Search
                | InputMode::Finder
                | InputMode::PinSelect
                | InputMode::LspMarketplace
                | InputMode::DiagnosticsList
                | InputMode::CodeActions
                | InputMode::SymbolInfo => InputAction::None,
            }
        }
        'V' => {
            state.reset_prefixes();
            match mode {
                InputMode::Normal => InputAction::SetMode(InputMode::VisualLine),
                InputMode::Visual => InputAction::SetMode(InputMode::VisualLine),
                InputMode::VisualBlock => InputAction::SetMode(InputMode::VisualLine),
                InputMode::VisualLine => InputAction::SetMode(InputMode::Normal),
                InputMode::Insert
                | InputMode::Command
                | InputMode::Search
                | InputMode::Finder
                | InputMode::PinSelect
                | InputMode::LspMarketplace
                | InputMode::DiagnosticsList
                | InputMode::CodeActions
                | InputMode::SymbolInfo => InputAction::None,
            }
        }
        'y' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.reset_prefixes();
            InputAction::YankSelectionPrivate
        }
        'd' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.reset_prefixes();
            InputAction::DeleteSelectionPrivate
        }
        'd' if mode == InputMode::Normal => {
            state.begin_operator(TextObjectOperator::Delete, None);
            InputAction::None
        }
        'y' if mode == InputMode::Normal && confirm_explorer_delete => {
            state.reset_prefixes();
            InputAction::ConfirmExplorerDelete
        }
        'y' if mode == InputMode::Normal => {
            state.begin_operator(TextObjectOperator::Yank, None);
            InputAction::None
        }
        'i' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.begin_operator(TextObjectOperator::Select, Some(TextObjectScope::Inner));
            InputAction::None
        }
        'a' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.begin_operator(TextObjectOperator::Select, Some(TextObjectScope::Around));
            InputAction::None
        }
        'c' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.reset_prefixes();
            InputAction::ChangeSelectionPrivate
        }
        'c' if mode == InputMode::Normal => {
            state.begin_operator(TextObjectOperator::Change, None);
            InputAction::None
        }
        'x' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.reset_prefixes();
            InputAction::DeleteSelectionNoYank
        }
        'x' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::DeleteCharNoYank
        }
        '~' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            InputAction::ToggleCase {
                count: state.take_count_or_1(),
            }
        }
        'f' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            let (count, count_explicit) = state.take_count();
            state.begin_search_motion(None, SearchMotionKind::Find, count, count_explicit);
            InputAction::None
        }
        't' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            let (count, count_explicit) = state.take_count();
            state.begin_search_motion(None, SearchMotionKind::Till, count, count_explicit);
            InputAction::None
        }
        'F' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            let (count, count_explicit) = state.take_count();
            state.begin_search_motion(None, SearchMotionKind::FindBefore, count, count_explicit);
            InputAction::None
        }
        'T' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            let (count, count_explicit) = state.take_count();
            state.begin_search_motion(None, SearchMotionKind::TillBefore, count, count_explicit);
            InputAction::None
        }
        'r' if matches!(
            mode,
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            state.begin_replace();
            InputAction::None
        }
        'p' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::PastePrivateRegister
        }
        'P' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::PastePrivateRegisterBefore
        }
        'J' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            InputAction::MoveVisualSelectionDown {
                count: state.take_count_or_1(),
            }
        }
        'J' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::JoinLineBelow
        }
        'K' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            InputAction::MoveVisualSelectionUp {
                count: state.take_count_or_1(),
            }
        }
        '\t' if matches!(
            mode,
            InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
        ) =>
        {
            InputAction::IndentVisualSelection {
                count: state.take_count_or_1(),
            }
        }
        ':' => {
            state.reset_prefixes();
            InputAction::EnterCommand
        }
        '/' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterSearch
        }
        'i' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::Insert)
        }
        'a' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::Append)
        }
        'I' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::InsertLineStart)
        }
        'A' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::EnterInsert(InsertKind::AppendLineEnd)
        }
        'D' if mode == InputMode::Normal => {
            let count = state.take_count_or_1();
            operate_target_action(
                state,
                TextObjectOperator::Delete,
                OperatorTarget::Motion {
                    motion: Motion::LineEnd,
                    count,
                },
            )
        }
        'o' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::OpenLineBelow
        }
        'O' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::OpenLineAbove
        }
        '-' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::SurfaceGoParent
        }
        _prefix if starts_sequence(mode, c, state.leader()) => {
            state.push_sequence_char(c);
            InputAction::None
        }
        'h' => InputAction::Motion {
            motion: Motion::Left,
            count: state.take_count_or_1(),
        },
        'j' => InputAction::Motion {
            motion: Motion::Down,
            count: state.take_count_or_1(),
        },
        'k' => InputAction::Motion {
            motion: Motion::Up,
            count: state.take_count_or_1(),
        },
        'l' => InputAction::Motion {
            motion: Motion::Right,
            count: state.take_count_or_1(),
        },
        'w' => InputAction::Motion {
            motion: Motion::WordStartAfter,
            count: state.take_count_or_1(),
        },
        'b' => InputAction::Motion {
            motion: Motion::WordStartBefore,
            count: state.take_count_or_1(),
        },
        'e' => InputAction::Motion {
            motion: Motion::WordEndAfter,
            count: state.take_count_or_1(),
        },
        '_' => InputAction::Motion {
            motion: Motion::LineFirstNonWhitespace,
            count: state.take_count_or_1(),
        },
        '%' => InputAction::Motion {
            motion: Motion::MatchDelimiter,
            count: state.take_count_or_1(),
        },
        '$' => InputAction::Motion {
            motion: Motion::LineEnd,
            count: state.take_count_or_1(),
        },
        'G' => InputAction::Motion {
            motion: Motion::FileEnd,
            count: state.take_count_or_1(),
        },
        'u' if mode == InputMode::Normal => {
            state.reset_prefixes();
            InputAction::Undo
        }
        _ => {
            state.reset_prefixes();
            InputAction::None
        }
    }
}

fn resolve_pending_operator(
    state: &mut InputState,
    c: char,
    pending: PendingOperator,
) -> Option<InputAction> {
    let action = match (pending.operator, pending.scope, c) {
        (TextObjectOperator::Delete, None, 'd') => Some(InputAction::DeleteCurrentLinePrivate {
            count: pending.count,
        }),
        (TextObjectOperator::Yank, None, 'y') => Some(InputAction::YankCurrentLinePrivate {
            count: pending.count,
        }),
        (TextObjectOperator::Change, None, 'c') => Some(InputAction::ChangeCurrentLinePrivate {
            count: pending.count,
        }),
        (operator, None, '0') => Some(InputAction::OperateTarget {
            operator,
            target: OperatorTarget::Motion {
                motion: Motion::LineStart,
                count: pending.count,
            },
        }),
        (operator, None, '_') => Some(InputAction::OperateTarget {
            operator,
            target: OperatorTarget::Motion {
                motion: Motion::LineFirstNonWhitespace,
                count: pending.count,
            },
        }),
        (operator, None, '$') => Some(InputAction::OperateTarget {
            operator,
            target: OperatorTarget::Motion {
                motion: Motion::LineEnd,
                count: pending.count,
            },
        }),
        (_, None, 'f') => {
            state.begin_search_motion(
                Some(pending),
                SearchMotionKind::Find,
                pending.count,
                pending.count_explicit,
            );
            Some(InputAction::None)
        }
        (_, None, 't') => {
            state.begin_search_motion(
                Some(pending),
                SearchMotionKind::Till,
                pending.count,
                pending.count_explicit,
            );
            Some(InputAction::None)
        }
        (_, None, 'F') => {
            state.begin_search_motion(
                Some(pending),
                SearchMotionKind::FindBefore,
                pending.count,
                pending.count_explicit,
            );
            Some(InputAction::None)
        }
        (_, None, 'T') => {
            state.begin_search_motion(
                Some(pending),
                SearchMotionKind::TillBefore,
                pending.count,
                pending.count_explicit,
            );
            Some(InputAction::None)
        }
        (_, None, 'i') => {
            state.pending_operator = Some(PendingOperator {
                scope: Some(TextObjectScope::Inner),
                ..pending
            });
            Some(InputAction::None)
        }
        (_, None, 'a') => {
            state.pending_operator = Some(PendingOperator {
                scope: Some(TextObjectScope::Around),
                ..pending
            });
            Some(InputAction::None)
        }
        (_, Some(scope), object) => {
            text_object_kind_from_char(object).map(|kind| InputAction::OperateTarget {
                operator: pending.operator,
                target: OperatorTarget::TextObject(TextObjectSpec {
                    scope,
                    kind,
                    count: pending.count,
                }),
            })
        }
        (_, None, motion_char) => {
            if let Some(motion) = operator_motion_from_input(state, motion_char) {
                Some(InputAction::OperateTarget {
                    operator: pending.operator,
                    target: OperatorTarget::Motion {
                        motion,
                        count: pending.count,
                    },
                })
            } else if operator_motion_sequence_has_children(state, motion_char) {
                state.push_sequence_char(motion_char);
                Some(InputAction::None)
            } else {
                None
            }
        }
    };

    let keep_pending_operator = matches!(action, Some(InputAction::None))
        && (matches!(
            (pending.operator, pending.scope, c),
            (_, None, 'i' | 'a' | 'f' | 't')
        ) || !state.pending_sequence.is_empty());

    if !matches!(action, Some(InputAction::None)) {
        state.pending_operator = None;
    } else if !keep_pending_operator {
        state.pending_operator = None;
    }

    action
}

fn operator_motion_from_input(state: &mut InputState, c: char) -> Option<Motion> {
    if !state.pending_sequence.is_empty() {
        let mut candidate = state.pending_sequence.clone();
        candidate.push(c);
        if let Some(motion) = motion_from_sequence(&candidate) {
            state.clear_sequence();
            return Some(motion);
        }
    }

    motion_from_char(c)
}

fn operator_motion_sequence_has_children(state: &InputState, c: char) -> bool {
    let mut candidate = state.pending_sequence.clone();
    candidate.push(c);
    motion_sequence_has_children(&candidate)
}

fn motion_from_char(c: char) -> Option<Motion> {
    match c {
        'h' => Some(Motion::Left),
        'j' => Some(Motion::Down),
        'k' => Some(Motion::Up),
        'l' => Some(Motion::Right),
        'w' => Some(Motion::WordStartAfter),
        'b' => Some(Motion::WordStartBefore),
        'e' => Some(Motion::WordEndAfter),
        '_' => Some(Motion::LineFirstNonWhitespace),
        '%' => Some(Motion::MatchDelimiter),
        '$' => Some(Motion::LineEnd),
        'G' => Some(Motion::FileEnd),
        _ => None,
    }
}

fn motion_from_sequence(sequence: &str) -> Option<Motion> {
    match sequence {
        "gg" => Some(Motion::FileStart),
        _ => None,
    }
}

fn motion_sequence_has_children(candidate: &str) -> bool {
    ["gg"]
        .into_iter()
        .any(|sequence| sequence.starts_with(candidate) && sequence.len() > candidate.len())
}

fn resolve_pending_search_motion(
    state: &mut InputState,
    c: char,
    pending: PendingSearchMotion,
) -> InputAction {
    state.pending_search_motion = None;
    let motion = match pending.kind {
        SearchMotionKind::Find => Motion::FindChar(c),
        SearchMotionKind::Till => Motion::TillChar(c),
        SearchMotionKind::FindBefore => Motion::FindCharBefore(c),
        SearchMotionKind::TillBefore => Motion::TillCharBefore(c),
    };

    if let Some(operator) = pending.operator {
        InputAction::OperateTarget {
            operator: operator.operator,
            target: OperatorTarget::Motion {
                motion,
                count: pending.count,
            },
        }
    } else {
        InputAction::Motion {
            motion,
            count: pending.count,
        }
    }
}

fn text_object_kind_from_char(c: char) -> Option<TextObjectKind> {
    match c {
        'w' => Some(TextObjectKind::Word),
        'W' => Some(TextObjectKind::BigWord),
        'p' => Some(TextObjectKind::Paragraph),
        '(' | ')' | 'b' => Some(TextObjectKind::Delimiter(DelimiterKind::Parentheses)),
        '[' | ']' => Some(TextObjectKind::Delimiter(DelimiterKind::Brackets)),
        '{' | '}' | 'B' => Some(TextObjectKind::Delimiter(DelimiterKind::Braces)),
        '\'' => Some(TextObjectKind::Delimiter(DelimiterKind::SingleQuotes)),
        '"' => Some(TextObjectKind::Delimiter(DelimiterKind::DoubleQuotes)),
        '`' => Some(TextObjectKind::Delimiter(DelimiterKind::Backticks)),
        _ => None,
    }
}

fn operate_target_action(
    state: &mut InputState,
    operator: TextObjectOperator,
    target: OperatorTarget,
) -> InputAction {
    state.reset_prefixes();
    InputAction::OperateTarget { operator, target }
}

fn sequence_bindings_for_mode(mode: InputMode) -> impl Iterator<Item = &'static SequenceBinding> {
    COMMON_SEQUENCE_BINDINGS.iter().chain(match mode {
        InputMode::Normal => NORMAL_SEQUENCE_BINDINGS.iter(),
        InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {
            VISUAL_SEQUENCE_BINDINGS.iter()
        }
        InputMode::Insert
        | InputMode::Command
        | InputMode::Search
        | InputMode::Finder
        | InputMode::PinSelect
        | InputMode::LspMarketplace
        | InputMode::DiagnosticsList
        | InputMode::CodeActions
        | InputMode::SymbolInfo => [].iter(),
    })
}

fn starts_sequence(mode: InputMode, c: char, leader: char) -> bool {
    sequence_bindings_for_mode(mode).any(|binding| {
        let sequence = builtin_sequence(binding.sequence, leader);
        sequence.chars().count() > 1 && sequence.starts_with(c)
    })
}

fn custom_sequence_starts(state: &InputState, mode: InputMode, c: char) -> bool {
    state.custom_bindings.iter().any(|binding| {
        binding.mode == mode
            && matches!(&binding.key, CustomKey::Sequence(sequence) if sequence.starts_with(c))
    })
}

fn custom_sequence_has_children(state: &InputState, mode: InputMode, candidate: &str) -> bool {
    state.custom_bindings.iter().any(|binding| {
        binding.mode == mode
            && matches!(&binding.key, CustomKey::Sequence(sequence)
                if sequence.starts_with(candidate) && sequence.len() > candidate.len())
    })
}

fn custom_sequence_action(
    state: &InputState,
    mode: InputMode,
    candidate: &str,
) -> Option<InputAction> {
    state
        .custom_bindings
        .iter()
        .find(|binding| {
            binding.mode == mode
                && matches!(&binding.key, CustomKey::Sequence(sequence) if sequence == candidate)
        })
        .map(|binding| binding.action.clone())
}

fn custom_special_action(state: &InputState, mode: InputMode, key: &str) -> Option<InputAction> {
    state
        .custom_bindings
        .iter()
        .find(|binding| {
            binding.mode == mode
                && matches!(&binding.key, CustomKey::Special(binding_key) if binding_key == key)
        })
        .map(|binding| binding.action.clone())
}

fn finish_custom_action(state: &mut InputState, mut action: InputAction) -> InputAction {
    if let InputAction::Motion { count, .. } = &mut action {
        *count = state.take_count_or_1();
    }
    state.reset_prefixes();
    action
}

fn resolve_pending_sequence(
    state: &mut InputState,
    mode: InputMode,
    c: char,
) -> Option<InputAction> {
    let mut candidate = state.pending_sequence.clone();
    candidate.push(c);

    if let Some(action) = custom_sequence_action(state, mode, &candidate) {
        return Some(finish_custom_action(state, action));
    }
    if custom_sequence_has_children(state, mode, &candidate) {
        state.pending_sequence = candidate;
        return Some(InputAction::None);
    }

    let leader = state.leader();
    let exact = sequence_bindings_for_mode(mode)
        .find(|binding| builtin_sequence(binding.sequence, leader) == candidate);
    let has_children = sequence_bindings_for_mode(mode).any(|binding| {
        let sequence = builtin_sequence(binding.sequence, leader);
        sequence.starts_with(&candidate) && sequence.len() > candidate.len()
    });

    if let Some(binding) = exact {
        state.clear_sequence();
        if has_children && binding.action.is_none() {
            state.pending_sequence = candidate;
            return Some(InputAction::None);
        }
        return Some(sequence_binding_action(state, binding));
    }

    if has_children {
        state.pending_sequence = candidate;
        return Some(InputAction::None);
    }

    let fallback = sequence_bindings_for_mode(mode)
        .find(|binding| builtin_sequence(binding.sequence, leader) == state.pending_sequence)
        .map(|binding| binding.fallback)
        .unwrap_or(PrefixFallback::Consume);
    state.clear_sequence();
    if fallback == PrefixFallback::Consume {
        state.pending_count = None;
        Some(InputAction::None)
    } else {
        None
    }
}

fn builtin_sequence(sequence: &str, leader: char) -> Cow<'_, str> {
    if leader == ' ' {
        Cow::Borrowed(sequence)
    } else {
        Cow::Owned(sequence.replace(' ', &leader.to_string()))
    }
}

fn modal_sequence_mode(mode: InputMode) -> bool {
    matches!(
        mode,
        InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
    )
}

fn normalize_special_key(keys: &str) -> anyhow::Result<String> {
    let inner = keys
        .strip_prefix('<')
        .and_then(|keys| keys.strip_suffix('>'))
        .ok_or_else(|| anyhow::anyhow!("invalid special key {keys:?}"))?;
    if inner.is_empty() || inner.contains(['<', '>']) {
        anyhow::bail!("invalid special key {keys:?}");
    }

    let normalized = inner.to_ascii_lowercase().replace('+', "-");
    let mut parts = normalized.split('-').collect::<Vec<_>>();
    let key = parts
        .pop()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| anyhow::anyhow!("special key {keys:?} is missing a key"))?;
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    for modifier in parts {
        let target = match modifier {
            "ctrl" | "control" => &mut ctrl,
            "alt" => &mut alt,
            "shift" => &mut shift,
            _ => anyhow::bail!("unknown modifier {modifier:?} in keybinding {keys:?}"),
        };
        if std::mem::replace(target, true) {
            anyhow::bail!("duplicate modifier {modifier:?} in keybinding {keys:?}");
        }
    }

    let mut canonical = String::from("<");
    if ctrl {
        canonical.push_str("ctrl-");
    }
    if alt {
        canonical.push_str("alt-");
    }
    if shift {
        canonical.push_str("shift-");
    }
    canonical.push_str(key);
    canonical.push('>');
    Ok(canonical)
}

fn ensure_unique_binding(
    configured: &mut HashSet<(InputMode, CustomKey)>,
    mode: InputMode,
    key: &CustomKey,
) -> anyhow::Result<()> {
    if !configured.insert((mode, key.clone())) {
        anyhow::bail!("duplicate keybinding {key:?} in mode {mode:?}");
    }
    Ok(())
}

fn reject_ambiguous_sequence_prefixes(bindings: &[CustomBinding]) -> anyhow::Result<()> {
    for binding in bindings {
        let CustomKey::Sequence(sequence) = &binding.key else {
            continue;
        };
        if bindings.iter().any(|candidate| {
            candidate.mode == binding.mode
                && matches!(&candidate.key, CustomKey::Sequence(candidate_sequence)
                    if candidate_sequence != sequence && candidate_sequence.starts_with(sequence))
        }) {
            anyhow::bail!(
                "keybinding {sequence:?} in mode {:?} is a prefix of another binding",
                binding.mode
            );
        }
    }
    Ok(())
}

fn input_mode_from_name(name: &str) -> Option<InputMode> {
    match name {
        "normal" => Some(InputMode::Normal),
        "insert" => Some(InputMode::Insert),
        "command" => Some(InputMode::Command),
        "search" => Some(InputMode::Search),
        "finder" => Some(InputMode::Finder),
        "pin_select" => Some(InputMode::PinSelect),
        "lsp_marketplace" => Some(InputMode::LspMarketplace),
        "diagnostics" => Some(InputMode::DiagnosticsList),
        "code_actions" => Some(InputMode::CodeActions),
        "symbol_info" => Some(InputMode::SymbolInfo),
        "visual" => Some(InputMode::Visual),
        "visual_line" => Some(InputMode::VisualLine),
        "visual_block" => Some(InputMode::VisualBlock),
        _ => None,
    }
}

fn configured_action(name: &str) -> anyhow::Result<(InputAction, &'static str)> {
    let action = match name {
        "open_explorer" => InputAction::OpenExplorer,
        "toggle_undo_tree" => InputAction::ToggleUndoTree,
        "open_finder" => InputAction::OpenFinder,
        "toggle_diagnostics" => InputAction::ToggleDiagnosticsList,
        "code_actions" => InputAction::TriggerCodeActions,
        "goto_definition" => InputAction::GotoDefinition,
        "symbol_info" => InputAction::TriggerSymbolInfo,
        "completion" => InputAction::TriggerCompletion,
        "undo" => InputAction::Undo,
        "redo" => InputAction::Redo,
        "move_left" => InputAction::Motion {
            motion: Motion::Left,
            count: 1,
        },
        "move_down" => InputAction::Motion {
            motion: Motion::Down,
            count: 1,
        },
        "move_up" => InputAction::Motion {
            motion: Motion::Up,
            count: 1,
        },
        "move_right" => InputAction::Motion {
            motion: Motion::Right,
            count: 1,
        },
        "word_forward" => InputAction::Motion {
            motion: Motion::WordStartAfter,
            count: 1,
        },
        "word_backward" => InputAction::Motion {
            motion: Motion::WordStartBefore,
            count: 1,
        },
        "line_start" => InputAction::Motion {
            motion: Motion::LineStart,
            count: 1,
        },
        "line_end" => InputAction::Motion {
            motion: Motion::LineEnd,
            count: 1,
        },
        "file_start" => InputAction::Motion {
            motion: Motion::FileStart,
            count: 1,
        },
        "file_end" => InputAction::Motion {
            motion: Motion::FileEnd,
            count: 1,
        },
        "insert" => InputAction::EnterInsert(InsertKind::Insert),
        "append" => InputAction::EnterInsert(InsertKind::Append),
        "insert_line_start" => InputAction::EnterInsert(InsertKind::InsertLineStart),
        "append_line_end" => InputAction::EnterInsert(InsertKind::AppendLineEnd),
        "open_line_below" => InputAction::OpenLineBelow,
        "open_line_above" => InputAction::OpenLineAbove,
        "delete_char" => InputAction::DeleteCharNoYank,
        "yank" => InputAction::YankSelectionPrivate,
        "delete" => InputAction::DeleteSelectionPrivate,
        "paste_system" => InputAction::PasteSystemClipboard,
        "yank_system" => InputAction::YankSelectionSystem,
        "paste" => InputAction::PastePrivateRegister,
        "paste_before" => InputAction::PastePrivateRegisterBefore,
        "command" => InputAction::EnterCommand,
        "search" => InputAction::EnterSearch,
        "split_horizontal" => InputAction::SplitHorizontal,
        "split_vertical" => InputAction::SplitVertical,
        "close_split" => InputAction::CloseSplit,
        "focus_left" => InputAction::SplitFocusLeft,
        "focus_down" => InputAction::SplitFocusDown,
        "focus_up" => InputAction::SplitFocusUp,
        "focus_right" => InputAction::SplitFocusRight,
        "centre_cursor" | "center_cursor" => InputAction::CenterCursorLine,
        "viewport_down" => InputAction::ViewportDownCenter,
        "viewport_up" => InputAction::ViewportUpCenter,
        "completion_next" => InputAction::CompletionMoveNext,
        "completion_previous" => InputAction::CompletionMovePrev,
        "completion_accept" => InputAction::CompletionAccept,
        "completion_cancel" => InputAction::CompletionCancel,
        "finder_next" => InputAction::FinderMoveNext,
        "finder_previous" => InputAction::FinderMovePrev,
        "finder_open" => InputAction::FinderEnter,
        "finder_cancel" => InputAction::FinderCancel,
        "surface_open" => InputAction::SurfaceOpenSelected,
        "surface_parent" => InputAction::SurfaceGoParent,
        "visual" => InputAction::SetMode(InputMode::Visual),
        "visual_line" => InputAction::SetMode(InputMode::VisualLine),
        "visual_block" => InputAction::SetMode(InputMode::VisualBlock),
        "pin_next" => InputAction::PinSelectorMoveNext,
        "pin_previous" => InputAction::PinSelectorMovePrev,
        "pin_open" => InputAction::PinSelectorOpenSelected,
        "pin_assign" => InputAction::PinSelectorAssign,
        "pin_delete" => InputAction::PinSelectorDeleteSelected,
        "marketplace_next" => InputAction::LspMarketplaceMoveNext,
        "marketplace_previous" => InputAction::LspMarketplaceMovePrev,
        "marketplace_install" => InputAction::LspMarketplaceInstallSelected,
        "marketplace_uninstall" => InputAction::LspMarketplaceUninstallSelected,
        "diagnostic_next" => InputAction::DiagnosticsListMoveNext,
        "diagnostic_previous" => InputAction::DiagnosticsListMovePrev,
        "diagnostic_open" => InputAction::DiagnosticsListOpenSelected,
        "code_action_next" => InputAction::CodeActionsMoveNext,
        "code_action_previous" => InputAction::CodeActionsMovePrev,
        "code_action_apply" => InputAction::CodeActionsApplySelected,
        "symbol_info_next" => InputAction::SymbolInfoMoveNext,
        "symbol_info_previous" => InputAction::SymbolInfoMovePrev,
        _ => anyhow::bail!("unknown keybinding action {name:?}"),
    };
    let description = input_action_description(&action);
    Ok((action, description))
}

fn input_action_description(action: &InputAction) -> &'static str {
    match action {
        InputAction::OpenExplorer => "Open explorer",
        InputAction::ToggleUndoTree => "Toggle undo tree",
        InputAction::OpenFinder => "Find files",
        InputAction::ToggleDiagnosticsList => "Toggle diagnostics",
        InputAction::TriggerCodeActions => "Code actions",
        InputAction::GotoDefinition => "Go to definition",
        InputAction::TriggerSymbolInfo => "Symbol information",
        InputAction::TriggerCompletion => "Completion",
        InputAction::Undo => "Undo",
        InputAction::Redo => "Redo",
        InputAction::Motion { motion, .. } => motion_description(motion),
        InputAction::EnterInsert(InsertKind::Insert) => "Insert",
        InputAction::EnterInsert(InsertKind::Append) => "Append",
        InputAction::EnterInsert(InsertKind::InsertLineStart) => "Insert at line start",
        InputAction::EnterInsert(InsertKind::AppendLineEnd) => "Append at line end",
        InputAction::OpenLineBelow => "Open line below",
        InputAction::OpenLineAbove => "Open line above",
        InputAction::DeleteCharNoYank => "Delete character",
        InputAction::YankSelectionPrivate => "Yank selection",
        InputAction::DeleteSelectionPrivate => "Delete selection",
        InputAction::PasteSystemClipboard => "Paste system clipboard",
        InputAction::YankSelectionSystem => "Yank to system clipboard",
        InputAction::PastePrivateRegister => "Paste",
        InputAction::PastePrivateRegisterBefore => "Paste before",
        InputAction::EnterCommand => "Command line",
        InputAction::EnterSearch => "Search",
        InputAction::SplitHorizontal => "Split horizontally",
        InputAction::SplitVertical => "Split vertically",
        InputAction::CloseSplit => "Close split",
        InputAction::SplitFocusLeft => "Focus split left",
        InputAction::SplitFocusDown => "Focus split down",
        InputAction::SplitFocusUp => "Focus split up",
        InputAction::SplitFocusRight => "Focus split right",
        InputAction::CenterCursorLine => "Centre cursor line",
        InputAction::ViewportDownCenter => "Viewport down",
        InputAction::ViewportUpCenter => "Viewport up",
        InputAction::CompletionMoveNext => "Next completion",
        InputAction::CompletionMovePrev => "Previous completion",
        InputAction::CompletionAccept => "Accept completion",
        InputAction::CompletionCancel => "Cancel completion",
        InputAction::FinderMoveNext => "Next result",
        InputAction::FinderMovePrev => "Previous result",
        InputAction::FinderEnter => "Open result",
        InputAction::FinderCancel => "Close finder",
        InputAction::SurfaceOpenSelected => "Open selection",
        InputAction::SurfaceGoParent => "Go to parent",
        InputAction::SetMode(InputMode::Visual) => "Visual mode",
        InputAction::SetMode(InputMode::VisualLine) => "Visual line mode",
        InputAction::SetMode(InputMode::VisualBlock) => "Visual block mode",
        InputAction::PinSelectorMoveNext => "Next pin",
        InputAction::PinSelectorMovePrev => "Previous pin",
        InputAction::PinSelectorOpenSelected => "Open pin",
        InputAction::PinSelectorAssign => "Assign pin",
        InputAction::PinSelectorDeleteSelected => "Delete pin",
        InputAction::LspMarketplaceMoveNext => "Next server",
        InputAction::LspMarketplaceMovePrev => "Previous server",
        InputAction::LspMarketplaceInstallSelected => "Install server",
        InputAction::LspMarketplaceUninstallSelected => "Uninstall server",
        InputAction::DiagnosticsListMoveNext => "Next diagnostic",
        InputAction::DiagnosticsListMovePrev => "Previous diagnostic",
        InputAction::DiagnosticsListOpenSelected => "Open diagnostic",
        InputAction::CodeActionsMoveNext => "Next code action",
        InputAction::CodeActionsMovePrev => "Previous code action",
        InputAction::CodeActionsApplySelected => "Apply code action",
        InputAction::SymbolInfoMoveNext => "Next symbol",
        InputAction::SymbolInfoMovePrev => "Previous symbol",
        _ => "Command",
    }
}

fn motion_description(motion: &Motion) -> &'static str {
    match motion {
        Motion::Left => "Move left",
        Motion::Down => "Move down",
        Motion::Up => "Move up",
        Motion::Right => "Move right",
        Motion::WordStartAfter => "Next word",
        Motion::WordStartBefore => "Previous word",
        Motion::WordEndAfter => "End of word",
        Motion::LineStart => "Start of line",
        Motion::LineFirstNonWhitespace => "First non-blank",
        Motion::LineEnd => "End of line",
        Motion::FileStart => "Start of file",
        Motion::FileEnd => "End of file",
        Motion::MatchDelimiter => "Matching delimiter",
        Motion::FindChar(_) => "Find character forwards",
        Motion::TillChar(_) => "Until character forwards",
        Motion::FindCharBefore(_) => "Find character backwards",
        Motion::TillCharBefore(_) => "Until character backwards",
    }
}

fn sequence_binding_action(state: &mut InputState, binding: &SequenceBinding) -> InputAction {
    match binding.action {
        Some(SequenceAction::OpenExplorer) => {
            state.reset_prefixes();
            InputAction::OpenExplorer
        }
        Some(SequenceAction::ToggleUndoTree) => {
            state.reset_prefixes();
            InputAction::ToggleUndoTree
        }
        Some(SequenceAction::OpenFinder) => {
            state.reset_prefixes();
            InputAction::OpenFinder
        }
        Some(SequenceAction::ToggleDiagnosticsList) => {
            state.reset_prefixes();
            InputAction::ToggleDiagnosticsList
        }
        Some(SequenceAction::TriggerCodeActions) => {
            state.reset_prefixes();
            InputAction::TriggerCodeActions
        }
        Some(SequenceAction::GotoDefinition) => {
            state.reset_prefixes();
            InputAction::GotoDefinition
        }
        Some(SequenceAction::YankSelectionSystem) => {
            state.reset_prefixes();
            InputAction::YankSelectionSystem
        }
        Some(SequenceAction::PasteSystemClipboard) => {
            state.reset_prefixes();
            InputAction::PasteSystemClipboard
        }
        Some(SequenceAction::FileStart) => {
            let _ = state.take_count_or_1();
            InputAction::Motion {
                motion: Motion::FileStart,
                count: 1,
            }
        }
        Some(SequenceAction::CenterCursorLine) => {
            state.reset_prefixes();
            InputAction::CenterCursorLine
        }
        None => InputAction::None,
    }
}

fn map_key_with_state(
    state: &mut InputState,
    mode: InputMode,
    confirm_explorer_delete: bool,
    key: KeyWithModifiers,
) -> InputAction {
    let mods = key.mods;
    let key = key.key;

    if let Some(token) = special_key_token(key, mods)
        && let Some(action) = custom_special_action(state, mode, &token)
    {
        return finish_custom_action(state, action);
    }

    match mode {
        InputMode::Insert => {
            if ctrl_key(mods, key, 'c') {
                state.reset_prefixes();
                return InputAction::CompletionCancel;
            }
            if ctrl_key(mods, key, 'i') {
                state.reset_prefixes();
                return InputAction::TriggerSymbolInfo;
            }
            if ctrl_shift_key(mods, key, 'k') {
                state.reset_prefixes();
                return InputAction::TriggerCompletion;
            }
            if ctrl_key(mods, key, 'k') {
                state.reset_prefixes();
                return InputAction::None;
            }
            if ctrl_key(mods, key, 'n') {
                state.reset_prefixes();
                return InputAction::CompletionMoveNext;
            }
            if ctrl_key(mods, key, 'p') {
                state.reset_prefixes();
                return InputAction::CompletionMovePrev;
            }
            if ctrl_key(mods, key, 'e') {
                state.reset_prefixes();
                return InputAction::CompletionCancel;
            }

            return match key {
                KeyKind::Escape if unmodified(mods) => InputAction::CompletionCancel,
                KeyKind::Backspace if unmodified(mods) => InputAction::Backspace,
                KeyKind::Enter if unmodified(mods) => InputAction::CompletionAccept,
                KeyKind::Tab if ctrl_only(mods) => InputAction::TriggerSymbolInfo,
                KeyKind::Tab if unmodified(mods) => InputAction::SnippetNext,

                KeyKind::Up if unmodified(mods) => InputAction::Motion {
                    motion: Motion::Up,
                    count: 1,
                },
                KeyKind::Down if unmodified(mods) => InputAction::Motion {
                    motion: Motion::Down,
                    count: 1,
                },
                KeyKind::Left if unmodified(mods) => InputAction::Motion {
                    motion: Motion::Left,
                    count: 1,
                },
                KeyKind::Right if unmodified(mods) => InputAction::Motion {
                    motion: Motion::Right,
                    count: 1,
                },

                KeyKind::Char(c) if text_mods(mods) => {
                    InputAction::InsertChar(replacement_char_from_key(c, mods))
                }

                _ => InputAction::None,
            };
        }

        InputMode::SymbolInfo => {
            if ctrl_key(mods, key, 'c') {
                state.reset_prefixes();
                return InputAction::SymbolInfoCancel;
            }

            return match key {
                KeyKind::Escape if unmodified(mods) => InputAction::SymbolInfoCancel,
                KeyKind::Up if unmodified(mods) => InputAction::SymbolInfoMovePrev,
                KeyKind::Down if unmodified(mods) => InputAction::SymbolInfoMoveNext,
                KeyKind::Char('j') if unmodified(mods) => InputAction::SymbolInfoMoveNext,
                KeyKind::Char('k') if unmodified(mods) => InputAction::SymbolInfoMovePrev,
                _ => InputAction::None,
            };
        }

        InputMode::CodeActions => {
            if ctrl_key(mods, key, 'c') {
                state.reset_prefixes();
                return InputAction::CodeActionsCancel;
            }

            return match key {
                KeyKind::Escape if unmodified(mods) => InputAction::CodeActionsCancel,
                KeyKind::Enter if unmodified(mods) => InputAction::CodeActionsApplySelected,
                KeyKind::Up if unmodified(mods) => InputAction::CodeActionsMovePrev,
                KeyKind::Down if unmodified(mods) => InputAction::CodeActionsMoveNext,
                KeyKind::Char('j') if unmodified(mods) => InputAction::CodeActionsMoveNext,
                KeyKind::Char('k') if unmodified(mods) => InputAction::CodeActionsMovePrev,
                _ => InputAction::None,
            };
        }

        InputMode::Command => {
            if ctrl_key(mods, key, 'c') {
                state.reset_prefixes();
                return InputAction::CommandCancel;
            }

            if ctrl_key(mods, key, 'p') {
                state.reset_prefixes();
                return InputAction::CommandHistoryPrev;
            }

            if ctrl_key(mods, key, 'n') {
                state.reset_prefixes();
                return InputAction::CommandHistoryNext;
            }

            return match key {
                KeyKind::Escape if unmodified(mods) => InputAction::CommandCancel,
                KeyKind::Backspace if unmodified(mods) => InputAction::CommandBackspace,
                KeyKind::Left if unmodified(mods) => InputAction::CommandMoveLeft,
                KeyKind::Right if unmodified(mods) => InputAction::CommandMoveRight,
                KeyKind::Up if unmodified(mods) => InputAction::CommandHistoryPrev,
                KeyKind::Down if unmodified(mods) => InputAction::CommandHistoryNext,
                KeyKind::Enter if unmodified(mods) => InputAction::CommandEnter,
                KeyKind::Char(c) if text_mods(mods) => {
                    InputAction::CommandChar(replacement_char_from_key(c, mods))
                }
                _ => InputAction::None,
            };
        }

        InputMode::Search => {
            if ctrl_key(mods, key, 'c') {
                state.reset_prefixes();
                return InputAction::SearchCancel;
            }

            return match key {
                KeyKind::Escape if unmodified(mods) => InputAction::SearchCancel,
                KeyKind::Backspace if unmodified(mods) => InputAction::SearchBackspace,
                KeyKind::Left if unmodified(mods) => InputAction::SearchMoveLeft,
                KeyKind::Right if unmodified(mods) => InputAction::SearchMoveRight,
                KeyKind::Enter if unmodified(mods) => InputAction::SearchEnter,
                KeyKind::Tab if unmodified(mods) => InputAction::SearchChar('\t'),
                KeyKind::Char(c) if text_mods(mods) => {
                    InputAction::SearchChar(replacement_char_from_key(c, mods))
                }
                _ => InputAction::None,
            };
        }

        InputMode::Finder => {
            if ctrl_key(mods, key, 'c') {
                state.reset_prefixes();
                return InputAction::FinderCancel;
            }

            if ctrl_shift_key(mods, key, 'p') {
                state.reset_prefixes();
                return InputAction::FinderBeginPin;
            }

            if ctrl_key(mods, key, 'n') {
                state.reset_prefixes();
                return InputAction::FinderMoveNext;
            }

            if ctrl_key(mods, key, 'p') {
                state.reset_prefixes();
                return InputAction::FinderMovePrev;
            }

            if ctrl_only(mods) && pin_slot_from_key(key).is_some() {
                state.reset_prefixes();
                return InputAction::OpenPinnedSlot {
                    slot: pin_slot_from_key(key).expect("finder slot"),
                };
            }

            return match key {
                KeyKind::Escape if unmodified(mods) => InputAction::FinderCancel,
                KeyKind::Backspace if unmodified(mods) => InputAction::FinderBackspace,
                KeyKind::Enter if unmodified(mods) => InputAction::FinderEnter,
                KeyKind::Left if unmodified(mods) => InputAction::FinderMoveLeft,
                KeyKind::Right if unmodified(mods) => InputAction::FinderMoveRight,
                KeyKind::Up if unmodified(mods) => InputAction::FinderMovePrev,
                KeyKind::Down if unmodified(mods) => InputAction::FinderMoveNext,
                KeyKind::Char(c) if text_mods(mods) => {
                    InputAction::FinderChar(replacement_char_from_key(c, mods))
                }
                _ => InputAction::None,
            };
        }

        InputMode::PinSelect => {
            if ctrl_key(mods, key, 'c') {
                state.reset_prefixes();
                return InputAction::PinSelectorCancel;
            }

            if unmodified(mods) && key == KeyKind::Char('p') {
                state.reset_prefixes();
                return InputAction::PinSelectorAssign;
            }

            if unmodified(mods) && key == KeyKind::Char('d') {
                state.reset_prefixes();
                return InputAction::PinSelectorDeleteSelected;
            }

            if ctrl_only(mods) && pin_slot_from_key(key).is_some() {
                state.reset_prefixes();
                return InputAction::AssignPinSlot {
                    slot: pin_slot_from_key(key).expect("pin selector slot"),
                };
            }

            if ctrl_key(mods, key, 'n') {
                state.reset_prefixes();
                return InputAction::PinSelectorMoveNext;
            }

            if ctrl_key(mods, key, 'p') {
                state.reset_prefixes();
                return InputAction::PinSelectorMovePrev;
            }

            if shift_key(mods, key, 'j') {
                state.reset_prefixes();
                return InputAction::PinSelectorReorderDown;
            }

            if shift_key(mods, key, 'k') {
                state.reset_prefixes();
                return InputAction::PinSelectorReorderUp;
            }

            if shift_only(mods) && key == KeyKind::Enter {
                state.reset_prefixes();
                return InputAction::PinSelectorAssign;
            }

            return match key {
                KeyKind::Escape if unmodified(mods) => InputAction::PinSelectorCancel,
                KeyKind::Enter if unmodified(mods) => InputAction::PinSelectorOpenSelected,
                KeyKind::Backspace if unmodified(mods) => InputAction::None,
                KeyKind::Up if unmodified(mods) => InputAction::PinSelectorMovePrev,
                KeyKind::Down if unmodified(mods) => InputAction::PinSelectorMoveNext,
                KeyKind::Char('j') if unmodified(mods) => InputAction::PinSelectorMoveNext,
                KeyKind::Char('k') if unmodified(mods) => InputAction::PinSelectorMovePrev,
                _ => InputAction::None,
            };
        }

        InputMode::LspMarketplace => {
            if ctrl_key(mods, key, 'c') {
                state.reset_prefixes();
                return InputAction::LspMarketplaceCancel;
            }

            return match key {
                KeyKind::Escape if unmodified(mods) => InputAction::LspMarketplaceCancel,
                KeyKind::Enter if unmodified(mods) => InputAction::None,
                KeyKind::Up if unmodified(mods) => InputAction::LspMarketplaceMovePrev,
                KeyKind::Down if unmodified(mods) => InputAction::LspMarketplaceMoveNext,
                KeyKind::Char('j') if unmodified(mods) => InputAction::LspMarketplaceMoveNext,
                KeyKind::Char('k') if unmodified(mods) => InputAction::LspMarketplaceMovePrev,
                KeyKind::Char('i') if unmodified(mods) => {
                    InputAction::LspMarketplaceInstallSelected
                }
                KeyKind::Char('u') if unmodified(mods) => {
                    InputAction::LspMarketplaceUninstallSelected
                }
                _ => InputAction::None,
            };
        }

        InputMode::DiagnosticsList => {
            if ctrl_key(mods, key, 'c') {
                state.reset_prefixes();
                return InputAction::DiagnosticsListCancel;
            }

            return match key {
                KeyKind::Escape if unmodified(mods) => InputAction::DiagnosticsListCancel,
                KeyKind::Char('a') if unmodified(mods) => InputAction::TriggerCodeActions,
                KeyKind::Enter if unmodified(mods) => InputAction::DiagnosticsListOpenSelected,
                KeyKind::Up if unmodified(mods) => InputAction::DiagnosticsListMovePrev,
                KeyKind::Down if unmodified(mods) => InputAction::DiagnosticsListMoveNext,
                KeyKind::Char('j') if unmodified(mods) => InputAction::DiagnosticsListMoveNext,
                KeyKind::Char('k') if unmodified(mods) => InputAction::DiagnosticsListMovePrev,
                _ => InputAction::None,
            };
        }

        InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock => {}
    }

    if matches!(
        mode,
        InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
    ) && let Some(action) = split_key_action(mods, key)
    {
        state.reset_prefixes();
        return action;
    }

    if let Some(pending_search_motion) = state.pending_search_motion {
        match key {
            KeyKind::Escape => {
                state.reset_prefixes();
                return if matches!(
                    mode,
                    InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
                ) {
                    InputAction::SetMode(InputMode::Normal)
                } else {
                    InputAction::None
                };
            }
            KeyKind::Tab => {
                state.pending_search_motion = None;
                return resolve_pending_search_motion(state, '\t', pending_search_motion);
            }
            KeyKind::Char(c) => {
                state.pending_search_motion = None;
                return resolve_pending_search_motion(
                    state,
                    replacement_char_from_key(c, mods),
                    pending_search_motion,
                );
            }
            _ => {
                state.reset_prefixes();
                return InputAction::None;
            }
        }
    }

    if state.pending_replace {
        match key {
            KeyKind::Escape => {
                state.reset_prefixes();
                return if matches!(
                    mode,
                    InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
                ) {
                    InputAction::SetMode(InputMode::Normal)
                } else {
                    InputAction::None
                };
            }
            KeyKind::Tab => {
                state.reset_prefixes();
                return InputAction::ReplaceChar('\t');
            }
            KeyKind::Char(c) => {
                state.reset_prefixes();
                return InputAction::ReplaceChar(replacement_char_from_key(c, mods));
            }
            _ => {
                state.reset_prefixes();
                return InputAction::None;
            }
        }
    }

    if mode == InputMode::Normal && ctrl_key(mods, key, 'r') {
        state.reset_prefixes();
        return InputAction::Redo;
    }

    if mode == InputMode::Normal && ctrl_key(mods, key, 'd') {
        state.reset_prefixes();
        return InputAction::ViewportDownCenter;
    }

    if mode == InputMode::Normal && ctrl_key(mods, key, 'u') {
        state.reset_prefixes();
        return InputAction::ViewportUpCenter;
    }

    if ctrl_shift_key(mods, key, 'p') {
        state.reset_prefixes();
        return InputAction::QuickPinCurrentFile;
    }

    if matches!(
        mode,
        InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
    ) && ctrl_key(mods, key, 'n')
    {
        state.reset_prefixes();
        return InputAction::RepeatSearch { forward: true };
    }

    if matches!(
        mode,
        InputMode::Normal | InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
    ) && ctrl_key(mods, key, 'p')
    {
        state.reset_prefixes();
        return InputAction::RepeatSearch { forward: false };
    }

    if matches!(
        mode,
        InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
    ) && ctrl_key(mods, key, 'c')
    {
        state.reset_prefixes();
        return InputAction::SetMode(InputMode::Normal);
    }

    if mode == InputMode::Normal && ctrl_key(mods, key, 'i') {
        state.reset_prefixes();
        return InputAction::TriggerSymbolInfo;
    }

    if ctrl_key(mods, key, 'v') {
        state.reset_prefixes();
        return match mode {
            InputMode::Normal | InputMode::Visual | InputMode::VisualLine => {
                InputAction::SetMode(InputMode::VisualBlock)
            }
            InputMode::VisualBlock => InputAction::SetMode(InputMode::Normal),
            InputMode::Insert
            | InputMode::Command
            | InputMode::Search
            | InputMode::Finder
            | InputMode::PinSelect
            | InputMode::LspMarketplace
            | InputMode::DiagnosticsList
            | InputMode::CodeActions
            | InputMode::SymbolInfo => InputAction::None,
        };
    }

    if ctrl_only(mods) && pin_slot_from_key(key).is_some() {
        state.reset_prefixes();
        return InputAction::OpenPinnedSlot {
            slot: pin_slot_from_key(key).expect("global pin slot"),
        };
    }

    // Detect `I`, `A`, etc. via key modifiers so terminal character event shape does not matter.
    if shift_only(mods) {
        if matches!(key, KeyKind::Char('I') | KeyKind::Char('i')) {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            return InputAction::EnterInsert(InsertKind::InsertLineStart);
        }
        if matches!(key, KeyKind::Char('A') | KeyKind::Char('a')) {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            return InputAction::EnterInsert(InsertKind::AppendLineEnd);
        }
        if matches!(key, KeyKind::Char('O') | KeyKind::Char('o')) {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            return InputAction::OpenLineAbove;
        }
        if matches!(key, KeyKind::Char('D') | KeyKind::Char('d')) {
            if mode != InputMode::Normal {
                state.reset_prefixes();
                return InputAction::None;
            }
            let count = state.take_count_or_1();
            state.reset_prefixes();
            return InputAction::OperateTarget {
                operator: TextObjectOperator::Delete,
                target: OperatorTarget::Motion {
                    motion: Motion::LineEnd,
                    count,
                },
            };
        }
        if matches!(key, KeyKind::Char('V') | KeyKind::Char('v')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'V');
        }
        if matches!(key, KeyKind::Char('P') | KeyKind::Char('p')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'P');
        }
        if matches!(key, KeyKind::Char('J') | KeyKind::Char('j')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'J');
        }
        if matches!(key, KeyKind::Char('K') | KeyKind::Char('k')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'K');
        }
        if matches!(key, KeyKind::Char('G') | KeyKind::Char('g')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'G');
        }
        if matches!(key, KeyKind::Char('F') | KeyKind::Char('f')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'F');
        }
        if matches!(key, KeyKind::Char('T') | KeyKind::Char('t')) {
            return modal_char_action(state, mode, confirm_explorer_delete, 'T');
        }
    }

    match key {
        KeyKind::Escape => {
            state.reset_prefixes();
            return if matches!(
                mode,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
            ) {
                InputAction::SetMode(InputMode::Normal)
            } else if mode == InputMode::Normal {
                InputAction::ClearSearch
            } else {
                InputAction::None
            };
        }
        KeyKind::Enter => {
            if mode != InputMode::Normal || !unmodified(mods) {
                state.reset_prefixes();
                return InputAction::None;
            }
            state.reset_prefixes();
            InputAction::SurfaceOpenSelected
        }
        KeyKind::Tab => {
            if !matches!(
                mode,
                InputMode::Visual | InputMode::VisualLine | InputMode::VisualBlock
            ) {
                state.reset_prefixes();
                return InputAction::None;
            }
            if shift_only(mods) {
                InputAction::OutdentVisualSelection {
                    count: state.take_count_or_1(),
                }
            } else if unmodified(mods) {
                InputAction::IndentVisualSelection {
                    count: state.take_count_or_1(),
                }
            } else {
                state.reset_prefixes();
                InputAction::None
            }
        }
        KeyKind::Up if unmodified(mods) => InputAction::Motion {
            motion: Motion::Up,
            count: state.take_count_or_1(),
        },
        KeyKind::Down if unmodified(mods) => InputAction::Motion {
            motion: Motion::Down,
            count: state.take_count_or_1(),
        },
        KeyKind::Left if unmodified(mods) => InputAction::Motion {
            motion: Motion::Left,
            count: state.take_count_or_1(),
        },
        KeyKind::Right if unmodified(mods) => InputAction::Motion {
            motion: Motion::Right,
            count: state.take_count_or_1(),
        },
        KeyKind::Char(c) if text_mods(mods) => modal_char_action(
            state,
            mode,
            confirm_explorer_delete,
            replacement_char_from_key(c, mods),
        ),
        _ => {
            state.reset_prefixes();
            InputAction::None
        }
    }
}

fn special_key_token(key: KeyKind, mods: KeyModifiers) -> Option<String> {
    if unmodified(mods) {
        return None;
    }
    let key = match key {
        KeyKind::Char(' ') => "space".to_string(),
        KeyKind::Char(c) => c.to_ascii_lowercase().to_string(),
        KeyKind::Tab => "tab".to_string(),
        KeyKind::Enter => "enter".to_string(),
        KeyKind::Escape => "escape".to_string(),
        KeyKind::Backspace => "backspace".to_string(),
        KeyKind::Delete => "delete".to_string(),
        KeyKind::Up => "up".to_string(),
        KeyKind::Down => "down".to_string(),
        KeyKind::Left => "left".to_string(),
        KeyKind::Right => "right".to_string(),
        KeyKind::Function(number) => format!("f{number}"),
        KeyKind::CapsLock => "capslock".to_string(),
    };
    let mut token = String::from("<");
    if mods.ctrl {
        token.push_str("ctrl-");
    }
    if mods.alt {
        token.push_str("alt-");
    }
    if mods.shift {
        token.push_str("shift-");
    }
    if mods.super_key {
        return None;
    }
    token.push_str(&key);
    token.push('>');
    Some(token)
}

fn replacement_char_from_key(c: char, mods: KeyModifiers) -> char {
    if !mods.shift {
        return c;
    }

    match c {
        'a'..='z' => c.to_ascii_uppercase(),
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '`' => '~',
        '\\' => '|',
        _ => c,
    }
}

fn unmodified(mods: KeyModifiers) -> bool {
    mods == KeyModifiers::none()
}

fn text_mods(mods: KeyModifiers) -> bool {
    unmodified(mods) || shift_only(mods)
}

fn ctrl_only(mods: KeyModifiers) -> bool {
    mods == KeyModifiers::ctrl()
}

fn shift_only(mods: KeyModifiers) -> bool {
    mods == KeyModifiers::shift()
}

fn ctrl_shift_only(mods: KeyModifiers) -> bool {
    mods == KeyModifiers {
        ctrl: true,
        shift: true,
        alt: false,
        super_key: false,
    }
}

fn shifted_char_key(key: KeyKind, c: char) -> bool {
    matches!(key, KeyKind::Char(actual) if actual == c || actual == c.to_ascii_uppercase())
}

fn ctrl_key(mods: KeyModifiers, key: KeyKind, c: char) -> bool {
    ctrl_only(mods) && key == KeyKind::Char(c)
}

fn shift_key(mods: KeyModifiers, key: KeyKind, c: char) -> bool {
    shift_only(mods) && shifted_char_key(key, c)
}

fn ctrl_shift_key(mods: KeyModifiers, key: KeyKind, c: char) -> bool {
    ctrl_shift_only(mods) && shifted_char_key(key, c)
}

fn split_key_action(mods: KeyModifiers, key: KeyKind) -> Option<InputAction> {
    if !ctrl_only(mods) {
        return None;
    }

    match key {
        KeyKind::Char('h') => Some(InputAction::SplitFocusLeft),
        KeyKind::Char('j') => Some(InputAction::SplitFocusDown),
        KeyKind::Char('k') => Some(InputAction::SplitFocusUp),
        KeyKind::Char('l') => Some(InputAction::SplitFocusRight),
        KeyKind::Char('-') => Some(InputAction::SplitHorizontal),
        KeyKind::Char('\\') => Some(InputAction::SplitVertical),
        KeyKind::Char('x') => Some(InputAction::CloseSplit),
        _ => None,
    }
}

fn pin_slot_from_key(key: KeyKind) -> Option<usize> {
    match key {
        KeyKind::Char('1') => Some(0),
        KeyKind::Char('2') => Some(1),
        KeyKind::Char('3') => Some(2),
        KeyKind::Char('4') => Some(3),
        KeyKind::Char('5') => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(key: KeyKind, mods: KeyModifiers) -> Event {
        Event::KeyWithModifiers(KeyWithModifiers { key, mods })
    }

    fn ctrl_shift() -> KeyModifiers {
        KeyModifiers {
            ctrl: true,
            shift: true,
            alt: false,
            super_key: false,
        }
    }

    fn map_event(mode: InputMode, event: &Event) -> InputAction {
        map_event_with_state(&mut InputState::new(), mode, event)
    }

    fn map_sequence(mode: InputMode, sequence: &str) -> InputAction {
        let mut state = InputState::new();
        let mut action = InputAction::None;
        for character in sequence.chars() {
            action = map_event_with_state(&mut state, mode, &Event::Character(character));
        }
        action
    }

    fn motion(motion: Motion, count: usize) -> InputAction {
        InputAction::Motion { motion, count }
    }

    fn motion_target(operator: TextObjectOperator, motion: Motion, count: usize) -> InputAction {
        InputAction::OperateTarget {
            operator,
            target: OperatorTarget::Motion { motion, count },
        }
    }

    fn text_object_target(
        operator: TextObjectOperator,
        scope: TextObjectScope,
        kind: TextObjectKind,
        count: usize,
    ) -> InputAction {
        InputAction::OperateTarget {
            operator,
            target: OperatorTarget::TextObject(TextObjectSpec { scope, kind, count }),
        }
    }

    #[test]
    fn normal_mode_maps_direct_actions() {
        let cases = [
            ("unused character", Event::Character('q'), InputAction::None),
            ("command", Event::Character(':'), InputAction::EnterCommand),
            (
                "line start",
                Event::Character('0'),
                motion(Motion::LineStart, 1),
            ),
            (
                "first non-blank",
                Event::Character('_'),
                motion(Motion::LineFirstNonWhitespace, 1),
            ),
            (
                "line end",
                Event::Character('$'),
                motion(Motion::LineEnd, 1),
            ),
            (
                "matching delimiter",
                Event::Character('%'),
                motion(Motion::MatchDelimiter, 1),
            ),
            ("search", Event::Character('/'), InputAction::EnterSearch),
            (
                "open below",
                Event::Character('o'),
                InputAction::OpenLineBelow,
            ),
            (
                "open above",
                Event::Character('O'),
                InputAction::OpenLineAbove,
            ),
            ("join", Event::Character('J'), InputAction::JoinLineBelow),
            (
                "visual",
                Event::Character('v'),
                InputAction::SetMode(InputMode::Visual),
            ),
            (
                "visual line",
                Event::Character('V'),
                InputAction::SetMode(InputMode::VisualLine),
            ),
            (
                "paste private",
                Event::Character('p'),
                InputAction::PastePrivateRegister,
            ),
            (
                "paste private before",
                Event::Character('P'),
                InputAction::PastePrivateRegisterBefore,
            ),
            ("undo", Event::Character('u'), InputAction::Undo),
            (
                "delete without yank",
                Event::Character('x'),
                InputAction::DeleteCharNoYank,
            ),
            (
                "toggle case",
                Event::Character('~'),
                InputAction::ToggleCase { count: 1 },
            ),
            (
                "open selected",
                Event::Enter,
                InputAction::SurfaceOpenSelected,
            ),
            (
                "go parent",
                Event::Character('-'),
                InputAction::SurfaceGoParent,
            ),
            ("clear search", Event::Escape, InputAction::ClearSearch),
        ];

        for (label, event, expected) in cases {
            assert_eq!(
                map_event(InputMode::Normal, &event),
                expected,
                "case: {label}"
            );
        }
    }

    #[test]
    fn terminal_key_shapes_are_normalized_consistently() {
        let cases = [
            (
                "shifted colon",
                key_event(KeyKind::Char(':'), KeyModifiers::shift()),
                InputAction::EnterCommand,
            ),
            (
                "shifted lowercase insert",
                key_event(KeyKind::Char('i'), KeyModifiers::shift()),
                InputAction::EnterInsert(InsertKind::InsertLineStart),
            ),
            (
                "shifted lowercase join",
                key_event(KeyKind::Char('j'), KeyModifiers::shift()),
                InputAction::JoinLineBelow,
            ),
            (
                "shifted lowercase visual",
                key_event(KeyKind::Char('v'), KeyModifiers::shift()),
                InputAction::SetMode(InputMode::VisualLine),
            ),
            (
                "shifted lowercase paste",
                key_event(KeyKind::Char('p'), KeyModifiers::shift()),
                InputAction::PastePrivateRegisterBefore,
            ),
            (
                "shifted number base",
                key_event(KeyKind::Char('4'), KeyModifiers::shift()),
                motion(Motion::LineEnd, 1),
            ),
            (
                "shifted backtick",
                key_event(KeyKind::Char('`'), KeyModifiers::shift()),
                InputAction::ToggleCase { count: 1 },
            ),
            (
                "unmodified enter",
                key_event(KeyKind::Enter, KeyModifiers::none()),
                InputAction::SurfaceOpenSelected,
            ),
            (
                "visual block",
                key_event(KeyKind::Char('v'), KeyModifiers::ctrl()),
                InputAction::SetMode(InputMode::VisualBlock),
            ),
            (
                "redo",
                key_event(KeyKind::Char('r'), KeyModifiers::ctrl()),
                InputAction::Redo,
            ),
            (
                "viewport down",
                key_event(KeyKind::Char('d'), KeyModifiers::ctrl()),
                InputAction::ViewportDownCenter,
            ),
            (
                "viewport up",
                key_event(KeyKind::Char('u'), KeyModifiers::ctrl()),
                InputAction::ViewportUpCenter,
            ),
            (
                "repeat search forwards",
                key_event(KeyKind::Char('n'), KeyModifiers::ctrl()),
                InputAction::RepeatSearch { forward: true },
            ),
            (
                "repeat search backwards",
                key_event(KeyKind::Char('p'), KeyModifiers::ctrl()),
                InputAction::RepeatSearch { forward: false },
            ),
            (
                "quick pin",
                key_event(KeyKind::Char('P'), ctrl_shift()),
                InputAction::QuickPinCurrentFile,
            ),
            (
                "open pin slot",
                key_event(KeyKind::Char('2'), KeyModifiers::ctrl()),
                InputAction::OpenPinnedSlot { slot: 1 },
            ),
            (
                "shifted pin symbol is not a slot",
                key_event(KeyKind::Char('!'), ctrl_shift()),
                InputAction::None,
            ),
        ];

        for (label, event, expected) in cases {
            assert_eq!(
                map_event(InputMode::Normal, &event),
                expected,
                "case: {label}"
            );
        }
    }

    #[test]
    fn default_sequences_dispatch_mode_specific_actions() {
        let cases = [
            (InputMode::Normal, "  ", InputAction::OpenFinder),
            (InputMode::Normal, " e", InputAction::OpenExplorer),
            (InputMode::Normal, " u", InputAction::ToggleUndoTree),
            (InputMode::Normal, " x", InputAction::ToggleDiagnosticsList),
            (InputMode::Normal, " ca", InputAction::TriggerCodeActions),
            (InputMode::Normal, "gd", InputAction::GotoDefinition),
            (InputMode::Normal, "gg", motion(Motion::FileStart, 1)),
            (InputMode::Normal, " p", InputAction::PasteSystemClipboard),
            (InputMode::Normal, "zz", InputAction::CenterCursorLine),
            (InputMode::Visual, " y", InputAction::YankSelectionSystem),
        ];

        for (mode, sequence, expected) in cases {
            assert_eq!(
                map_sequence(mode, sequence),
                expected,
                "sequence: {sequence:?}"
            );
        }
    }

    #[test]
    fn counts_apply_once_and_clear_at_sequence_boundaries() {
        assert_eq!(
            map_sequence(InputMode::Normal, "3w"),
            motion(Motion::WordStartAfter, 3)
        );
        assert_eq!(
            map_sequence(InputMode::Normal, "2D"),
            motion_target(TextObjectOperator::Delete, Motion::LineEnd, 2)
        );
        assert_eq!(
            map_sequence(InputMode::Normal, "3~"),
            InputAction::ToggleCase { count: 3 }
        );

        let mut state = InputState::new();
        for character in "12gg".chars() {
            let _ =
                map_event_with_state(&mut state, InputMode::Normal, &Event::Character(character));
        }
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Normal, &Event::Character('j')),
            motion(Motion::Down, 1)
        );

        let mut state = InputState::new();
        for character in "2 x".chars() {
            let _ =
                map_event_with_state(&mut state, InputMode::Normal, &Event::Character(character));
        }
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Normal, &Event::Character('j')),
            motion(Motion::Down, 1)
        );
    }

    #[test]
    fn operators_resolve_motion_and_line_targets() {
        let cases = [
            (
                "d%",
                motion_target(TextObjectOperator::Delete, Motion::MatchDelimiter, 1),
            ),
            (
                "d$",
                motion_target(TextObjectOperator::Delete, Motion::LineEnd, 1),
            ),
            (
                "dw",
                motion_target(TextObjectOperator::Delete, Motion::WordStartAfter, 1),
            ),
            (
                "dgg",
                motion_target(TextObjectOperator::Delete, Motion::FileStart, 1),
            ),
            (
                "d0",
                motion_target(TextObjectOperator::Delete, Motion::LineStart, 1),
            ),
            ("dd", InputAction::DeleteCurrentLinePrivate { count: 1 }),
            ("yy", InputAction::YankCurrentLinePrivate { count: 1 }),
            ("cc", InputAction::ChangeCurrentLinePrivate { count: 1 }),
        ];

        for (sequence, expected) in cases {
            assert_eq!(
                map_sequence(InputMode::Normal, sequence),
                expected,
                "sequence: {sequence}"
            );
        }
    }

    #[test]
    fn character_searches_resolve_plain_and_operator_motions() {
        let plain_cases = [
            ("fx", Motion::FindChar('x')),
            ("tx", Motion::TillChar('x')),
            ("Fx", Motion::FindCharBefore('x')),
            ("Tx", Motion::TillCharBefore('x')),
        ];
        for (sequence, expected_motion) in plain_cases {
            assert_eq!(
                map_sequence(InputMode::Normal, sequence),
                motion(expected_motion, 1),
                "sequence: {sequence}"
            );
        }

        let operator_cases = [
            ("dtx", TextObjectOperator::Delete, Motion::TillChar('x')),
            ("cfx", TextObjectOperator::Change, Motion::FindChar('x')),
            (
                "dTx",
                TextObjectOperator::Delete,
                Motion::TillCharBefore('x'),
            ),
            (
                "cFx",
                TextObjectOperator::Change,
                Motion::FindCharBefore('x'),
            ),
        ];
        for (sequence, operator, expected_motion) in operator_cases {
            assert_eq!(
                map_sequence(InputMode::Normal, sequence),
                motion_target(operator, expected_motion, 1),
                "sequence: {sequence}"
            );
        }

        let mut state = InputState::new();
        let _ = map_event_with_state(
            &mut state,
            InputMode::Normal,
            &key_event(KeyKind::Char('f'), KeyModifiers::shift()),
        );
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Normal, &Event::Character('x')),
            motion(Motion::FindCharBefore('x'), 1)
        );
    }

    #[test]
    fn replacement_state_normalizes_characters_and_cancels_cleanly() {
        assert_eq!(
            map_sequence(InputMode::Normal, "rx"),
            InputAction::ReplaceChar('x')
        );

        let replacements = [
            (
                key_event(KeyKind::Char('D'), KeyModifiers::shift()),
                InputAction::ReplaceChar('D'),
            ),
            (
                key_event(KeyKind::Char('$'), KeyModifiers::shift()),
                InputAction::ReplaceChar('$'),
            ),
            (
                key_event(KeyKind::Char('4'), KeyModifiers::shift()),
                InputAction::ReplaceChar('$'),
            ),
            (
                key_event(KeyKind::Tab, KeyModifiers::none()),
                InputAction::ReplaceChar('\t'),
            ),
        ];
        for (event, expected) in replacements {
            let mut state = InputState::new();
            let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('r'));
            assert_eq!(
                map_event_with_state(&mut state, InputMode::Normal, &event),
                expected
            );
        }

        for (event, expected) in [
            (Event::Escape, InputAction::None),
            (Event::Backspace, InputAction::None),
            (Event::Enter, InputAction::SurfaceOpenSelected),
        ] {
            let mut state = InputState::new();
            let _ = map_event_with_state(&mut state, InputMode::Normal, &Event::Character('r'));
            assert_eq!(
                map_event_with_state(&mut state, InputMode::Normal, &event),
                expected
            );
            assert_eq!(
                map_event_with_state(&mut state, InputMode::Normal, &Event::Character('q')),
                InputAction::None
            );
        }
    }

    #[test]
    fn text_objects_resolve_operator_and_visual_targets() {
        let cases = [
            (
                InputMode::Normal,
                "diw",
                text_object_target(
                    TextObjectOperator::Delete,
                    TextObjectScope::Inner,
                    TextObjectKind::Word,
                    1,
                ),
            ),
            (
                InputMode::Normal,
                "cap",
                text_object_target(
                    TextObjectOperator::Change,
                    TextObjectScope::Around,
                    TextObjectKind::Paragraph,
                    1,
                ),
            ),
            (
                InputMode::Normal,
                "2ci]",
                text_object_target(
                    TextObjectOperator::Change,
                    TextObjectScope::Inner,
                    TextObjectKind::Delimiter(DelimiterKind::Brackets),
                    2,
                ),
            ),
            (
                InputMode::Normal,
                "ci\"",
                text_object_target(
                    TextObjectOperator::Change,
                    TextObjectScope::Inner,
                    TextObjectKind::Delimiter(DelimiterKind::DoubleQuotes),
                    1,
                ),
            ),
            (
                InputMode::Visual,
                "iw",
                text_object_target(
                    TextObjectOperator::Select,
                    TextObjectScope::Inner,
                    TextObjectKind::Word,
                    1,
                ),
            ),
            (
                InputMode::Visual,
                "a[",
                text_object_target(
                    TextObjectOperator::Select,
                    TextObjectScope::Around,
                    TextObjectKind::Delimiter(DelimiterKind::Brackets),
                    1,
                ),
            ),
            (
                InputMode::Visual,
                "iW",
                text_object_target(
                    TextObjectOperator::Select,
                    TextObjectScope::Inner,
                    TextObjectKind::BigWord,
                    1,
                ),
            ),
        ];

        for (mode, sequence, expected) in cases {
            assert_eq!(
                map_sequence(mode, sequence),
                expected,
                "sequence: {sequence}"
            );
        }
    }

    #[test]
    fn visual_modes_map_selection_actions_and_exit() {
        let cases = [
            (
                InputMode::Visual,
                Event::Character('y'),
                InputAction::YankSelectionPrivate,
            ),
            (
                InputMode::Visual,
                Event::Character('d'),
                InputAction::DeleteSelectionPrivate,
            ),
            (
                InputMode::Visual,
                Event::Character('c'),
                InputAction::ChangeSelectionPrivate,
            ),
            (
                InputMode::Visual,
                Event::Character('x'),
                InputAction::DeleteSelectionNoYank,
            ),
            (
                InputMode::Visual,
                Event::Character('~'),
                InputAction::ToggleCase { count: 1 },
            ),
            (
                InputMode::Visual,
                Event::Character('J'),
                InputAction::MoveVisualSelectionDown { count: 1 },
            ),
            (
                InputMode::Visual,
                Event::Character('K'),
                InputAction::MoveVisualSelectionUp { count: 1 },
            ),
            (
                InputMode::Visual,
                key_event(KeyKind::Tab, KeyModifiers::none()),
                InputAction::IndentVisualSelection { count: 1 },
            ),
            (
                InputMode::Visual,
                key_event(KeyKind::Tab, KeyModifiers::shift()),
                InputAction::OutdentVisualSelection { count: 1 },
            ),
            (
                InputMode::Visual,
                key_event(KeyKind::Char('v'), KeyModifiers::ctrl()),
                InputAction::SetMode(InputMode::VisualBlock),
            ),
            (
                InputMode::Visual,
                Event::Escape,
                InputAction::SetMode(InputMode::Normal),
            ),
            (
                InputMode::VisualLine,
                key_event(KeyKind::Char('c'), KeyModifiers::ctrl()),
                InputAction::SetMode(InputMode::Normal),
            ),
            (
                InputMode::VisualBlock,
                key_event(KeyKind::Escape, KeyModifiers::none()),
                InputAction::SetMode(InputMode::Normal),
            ),
        ];

        for (mode, event, expected) in cases {
            assert_eq!(map_event(mode, &event), expected, "event: {event:?}");
        }
    }

    #[test]
    fn editable_modes_map_text_navigation_and_cancellation() {
        let cases = [
            (InputMode::Insert, Event::Character('\0'), InputAction::None),
            (
                InputMode::Insert,
                key_event(KeyKind::Tab, KeyModifiers::none()),
                InputAction::SnippetNext,
            ),
            (
                InputMode::Insert,
                key_event(KeyKind::Char('k'), ctrl_shift()),
                InputAction::TriggerCompletion,
            ),
            (
                InputMode::Insert,
                key_event(KeyKind::Char('k'), KeyModifiers::ctrl()),
                InputAction::None,
            ),
            (
                InputMode::Insert,
                Event::Keybind(KeybindAction::Custom("trigger-completion".to_string())),
                InputAction::TriggerCompletion,
            ),
            (
                InputMode::Insert,
                key_event(KeyKind::Char('e'), KeyModifiers::ctrl()),
                InputAction::CompletionCancel,
            ),
            (
                InputMode::Insert,
                Event::Escape,
                InputAction::CompletionCancel,
            ),
            (
                InputMode::Insert,
                key_event(KeyKind::Char('-'), KeyModifiers::shift()),
                InputAction::InsertChar('_'),
            ),
            (
                InputMode::Command,
                key_event(KeyKind::Char('-'), KeyModifiers::shift()),
                InputAction::CommandChar('_'),
            ),
            (
                InputMode::Command,
                key_event(KeyKind::Up, KeyModifiers::none()),
                InputAction::CommandHistoryPrev,
            ),
            (
                InputMode::Command,
                key_event(KeyKind::Char('n'), KeyModifiers::ctrl()),
                InputAction::CommandHistoryNext,
            ),
            (
                InputMode::Command,
                key_event(KeyKind::Char('c'), KeyModifiers::ctrl()),
                InputAction::CommandCancel,
            ),
            (
                InputMode::Search,
                Event::Character('x'),
                InputAction::SearchChar('x'),
            ),
            (
                InputMode::Search,
                Event::Backspace,
                InputAction::SearchBackspace,
            ),
            (InputMode::Search, Event::Enter, InputAction::SearchEnter),
        ];

        for (mode, event, expected) in cases {
            assert_eq!(map_event(mode, &event), expected, "event: {event:?}");
        }
    }

    #[test]
    fn popup_modes_map_navigation_selection_and_cancellation() {
        let cases = [
            (
                InputMode::Finder,
                key_event(KeyKind::Char('n'), KeyModifiers::ctrl()),
                InputAction::FinderMoveNext,
            ),
            (
                InputMode::Finder,
                key_event(KeyKind::Char('p'), KeyModifiers::ctrl()),
                InputAction::FinderMovePrev,
            ),
            (
                InputMode::Finder,
                key_event(KeyKind::Char('P'), ctrl_shift()),
                InputAction::FinderBeginPin,
            ),
            (
                InputMode::PinSelect,
                key_event(KeyKind::Char('3'), KeyModifiers::ctrl()),
                InputAction::AssignPinSlot { slot: 2 },
            ),
            (
                InputMode::PinSelect,
                key_event(KeyKind::Char('j'), KeyModifiers::shift()),
                InputAction::PinSelectorReorderDown,
            ),
            (
                InputMode::PinSelect,
                Event::Character('p'),
                InputAction::PinSelectorAssign,
            ),
            (
                InputMode::PinSelect,
                Event::Character('d'),
                InputAction::PinSelectorDeleteSelected,
            ),
            (
                InputMode::PinSelect,
                Event::Enter,
                InputAction::PinSelectorOpenSelected,
            ),
            (
                InputMode::LspMarketplace,
                Event::Character('i'),
                InputAction::LspMarketplaceInstallSelected,
            ),
            (
                InputMode::LspMarketplace,
                Event::Character('u'),
                InputAction::LspMarketplaceUninstallSelected,
            ),
            (
                InputMode::DiagnosticsList,
                Event::Character('a'),
                InputAction::TriggerCodeActions,
            ),
            (
                InputMode::DiagnosticsList,
                Event::Character('f'),
                InputAction::None,
            ),
            (
                InputMode::CodeActions,
                Event::Enter,
                InputAction::CodeActionsApplySelected,
            ),
            (
                InputMode::SymbolInfo,
                Event::Character('j'),
                InputAction::SymbolInfoMoveNext,
            ),
            (
                InputMode::SymbolInfo,
                key_event(KeyKind::Up, KeyModifiers::none()),
                InputAction::SymbolInfoMovePrev,
            ),
            (
                InputMode::SymbolInfo,
                Event::Escape,
                InputAction::SymbolInfoCancel,
            ),
        ];

        for (mode, event, expected) in cases {
            assert_eq!(map_event(mode, &event), expected, "event: {event:?}");
        }
    }

    #[test]
    fn explorer_confirmation_requires_confirmation_context() {
        let mut state = InputState::new();
        assert_eq!(
            map_event_with_context(&mut state, InputMode::Normal, true, &Event::Character('y'),),
            InputAction::ConfirmExplorerDelete
        );
        assert_eq!(
            map_event(InputMode::Normal, &Event::Character('y')),
            InputAction::None
        );
    }

    #[test]
    fn which_key_entries_follow_the_active_mode() {
        let mut state = InputState::new();
        let _ = map_event_with_state(&mut state, InputMode::Visual, &Event::Character(' '));
        let popup = state
            .which_key_popup(
                InputMode::Visual,
                Instant::now() + DEFAULT_WHICH_KEY_DELAY,
                DEFAULT_WHICH_KEY_DELAY,
            )
            .expect("visual leader should expose which-key entries");

        assert_eq!(popup.prefix, "<leader>");
        assert!(
            popup.entries.iter().any(|entry| {
                entry.key == "y" && entry.description == "Yank to system clipboard"
            })
        );
    }

    #[test]
    fn configured_bindings_dispatch_by_leader_mode_and_count() {
        let mut state = InputState::new();
        let bindings = BTreeMap::from([
            (
                "normal".to_string(),
                BTreeMap::from([
                    ("open_finder".to_string(), "<leader>f".to_string()),
                    ("undo".to_string(), "<ctrl-g>".to_string()),
                    ("move_right".to_string(), "q".to_string()),
                ]),
            ),
            (
                "insert".to_string(),
                BTreeMap::from([("completion".to_string(), "<ctrl-g>".to_string())]),
            ),
        ]);
        state
            .configure(',', &bindings)
            .expect("bindings should configure");

        assert_eq!(
            map_event_with_state(&mut state, InputMode::Normal, &Event::Character(',')),
            InputAction::None
        );
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Normal, &Event::Character('f')),
            InputAction::OpenFinder
        );

        let configured_key =
            Event::Keybind(KeybindAction::Custom("redox-key:<ctrl-g>".to_string()));
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Normal, &configured_key),
            InputAction::Undo
        );
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Insert, &configured_key),
            InputAction::TriggerCompletion
        );

        assert_eq!(
            map_event_with_state(&mut state, InputMode::Normal, &Event::Character('3')),
            InputAction::None
        );
        assert_eq!(
            map_event_with_state(&mut state, InputMode::Normal, &Event::Character('q')),
            motion(Motion::Right, 3)
        );
    }

    #[test]
    fn configured_bindings_reject_invalid_sequences() {
        let mut state = InputState::new();
        let ambiguous = BTreeMap::from([(
            "normal".to_string(),
            BTreeMap::from([
                ("undo".to_string(), "g".to_string()),
                ("redo".to_string(), "gg".to_string()),
            ]),
        )]);
        assert!(state.configure(' ', &ambiguous).is_err());

        let unsupported = BTreeMap::from([(
            "insert".to_string(),
            BTreeMap::from([("completion".to_string(), "jj".to_string())]),
        )]);
        assert!(state.configure(' ', &unsupported).is_err());

        let cycle = ConfiguredBinding {
            mode: "normal".to_string(),
            keys: "Q".to_string(),
            target: ConfiguredBindingTarget::Sequence("Q".to_string()),
            description: "Cycle".to_string(),
        };
        assert!(state.configure_custom_bindings(&[cycle]).is_err());
    }
}
