use crate::desktop::{
    i18n::{fill, translate, Lang, Table},
    ux::seams::{BackgroundService, KeepAlive},
};

#[derive(Debug, PartialEq, Eq)]
pub enum ClosePlan {
    CloseNow,
    Continue { sessions: Vec<String> },
    AskStop { sessions: Vec<String> },
}
pub fn close_plan(running_here: &[String], bg: Option<&dyn BackgroundService>) -> ClosePlan {
    if running_here.is_empty() {
        ClosePlan::CloseNow
    } else if bg.is_some_and(|bg| matches!(bg.status(), KeepAlive::Available)) {
        ClosePlan::Continue {
            sessions: running_here.to_vec(),
        }
    } else {
        ClosePlan::AskStop {
            sessions: running_here.to_vec(),
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct CloseCopy {
    pub title: String,
    pub description: String,
    pub confirm: String,
    pub cancel: String,
}
const T: Table = &[
    ("Work continues in the background", "工作會在背景繼續"),
    ("\"{}\" keeps working in the background after the window closes. Reopen Chronos to see it again.", "視窗關閉後，「{}」會在背景繼續。重新開啟 Chronos 即可回到這裡。"),
    ("{} wishes keep working in the background after the window closes. Reopen Chronos to see them again.", "視窗關閉後，{} 個願望會在背景繼續。重新開啟 Chronos 即可回到這裡。"),
    ("Close window", "關閉視窗"), ("Keep window open", "保持開啟"),
];
pub fn close_copy(plan: &ClosePlan, wishes: &[String], lang: Lang) -> Option<CloseCopy> {
    let ClosePlan::Continue { sessions } = plan else {
        return None;
    };
    let t = |key| translate(lang, T, key);
    let description = if sessions.len() == 1 {
        fill(t("\"{}\" keeps working in the background after the window closes. Reopen Chronos to see it again."), &[wishes.first().map(String::as_str).unwrap_or("")])
    } else {
        fill(t("{} wishes keep working in the background after the window closes. Reopen Chronos to see them again."), &[&sessions.len().to_string()])
    };
    Some(CloseCopy {
        title: t("Work continues in the background").into(),
        description,
        confirm: t("Close window").into(),
        cancel: t("Keep window open").into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::ux::seams::FakeBackground;
    use std::prelude::v1::test;
    #[test]
    fn plans_and_copy() {
        let ids = vec!["s".to_string()];
        let yes = FakeBackground::new(true);
        let no = FakeBackground::new(false);
        assert_eq!(close_plan(&[], Some(&yes)), ClosePlan::CloseNow);
        for bg in [None, Some(&no as &dyn BackgroundService)] {
            assert_eq!(
                close_plan(&ids, bg),
                ClosePlan::AskStop {
                    sessions: ids.clone()
                }
            );
        }
        let plan = close_plan(&ids, Some(&yes));
        assert_eq!(close_copy(&plan, &["Wish".into()], Lang::En).unwrap().description, "\"Wish\" keeps working in the background after the window closes. Reopen Chronos to see it again.");
        assert_eq!(
            close_copy(&plan, &["願望".into()], Lang::ZhTw)
                .unwrap()
                .description,
            "視窗關閉後，「願望」會在背景繼續。重新開啟 Chronos 即可回到這裡。"
        );
        let en = close_copy(&plan, &["Wish".into()], Lang::En).unwrap();
        assert_eq!(
            (en.title.as_str(), en.confirm.as_str(), en.cancel.as_str()),
            (
                "Work continues in the background",
                "Close window",
                "Keep window open"
            )
        );
        let zh = close_copy(&plan, &["願望".into()], Lang::ZhTw).unwrap();
        assert_eq!(
            (zh.title.as_str(), zh.confirm.as_str(), zh.cancel.as_str()),
            ("工作會在背景繼續", "關閉視窗", "保持開啟")
        );
        let plural = ClosePlan::Continue {
            sessions: vec!["a".into(), "b".into()],
        };
        assert_eq!(close_copy(&plural, &[], Lang::En).unwrap().description, "2 wishes keep working in the background after the window closes. Reopen Chronos to see them again.");
        assert_eq!(
            close_copy(&plural, &[], Lang::ZhTw).unwrap().description,
            "視窗關閉後，2 個願望會在背景繼續。重新開啟 Chronos 即可回到這裡。"
        );
    }
}
