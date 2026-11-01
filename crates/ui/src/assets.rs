//! The asset source the app registers: the component library's default icons plus the Lucide
//! icons ThinkRail's own components use (selected at compile time, nothing downloaded).

use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString, assets::Assets};

gpui_kit::assets::icon_assets!(
    Extra,
    [
        Folder, FolderOpen, FolderGit2, GitBranch, GitCompare, GitCommitHorizontal, Plus, X, ChevronRight,
        ChevronDown, ChevronUp, Settings, SquareTerminal, RotateCcwClock, File, FileText, Files, FileDiff, FilePlus, ScrollText,
        FileMinus, MessageSquare, MessageCirclePlus, List, ListTree, ListChecks, SendHorizontal, ArrowUp, Square,
        Circle, CircleCheck, CircleX, CircleQuestionMark, Check, Ban, LoaderCircle, TriangleAlert, CircleAlert, Copy,
        PanelLeft, Brain, Wrench, Clock, Undo2, Pencil, Trash, Eye, Search, Play, Pause, Minus, Bot, User,
        Ellipsis, RefreshCw, SquarePen, Columns2, Rows2, ShieldCheck, ShieldAlert, Plug, KeyRound, Star, TextWrap,
        GitPullRequest, Settings2, SlidersHorizontal, ChevronsUpDown, ChevronsDownUp, EyeOff, ExternalLink, Sparkles
    ]
);

#[derive(Debug, Clone, Copy, Default)]
pub struct UiAssets;

impl AssetSource for UiAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        match Extra.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut all = Extra.list(path)?;
        all.extend(Assets.list(path)?);
        all.sort();
        all.dedup();
        Ok(all)
    }
}
