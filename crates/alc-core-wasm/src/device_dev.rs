//! dev端末 (開発用の鍵) の印 (Refs ippoan/alc-app#387)。
//!
//! auth-worker は検証済みの要求にだけ `X-Device-Dev: 1` を付けて backend へ渡す。
//! backend はその印を DB の接続の設定 `app.device_dev` に通し、列の既定値と RLS が
//! dev の行と本番の行を分ける (alc-migrations 153)。
//!
//! 端末の鍵の経路は、鍵の役割も `X-Device-Role: <role>` で渡してくる。dev の運行管理者の
//! 鍵にだけ開く口 (点呼の判定) が、dev の印と合わせて見る。
//!
//! ここに置くのは tokio に依らない部分だけ — ヘッダを読む関数 (欄ごとに 1 つ) と、
//! 要求 1 件ぶんの印の入れ物。入れ物を要求のスコープ (task-local) に結ぶのは
//! `alc-core` の `device_dev`。

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use axum::http::HeaderMap;

pub const DEVICE_DEV_HEADER: &str = "X-Device-Dev";
pub const DEVICE_ROLE_HEADER: &str = "X-Device-Role";
/// 運行管理者用の鍵の role (auth-worker の端末 JWT の claim と同じ値)。
pub const DEVICE_ROLE_TENKO_MANAGER: &str = "device-tenko-manager";

/// `X-Device-Dev` を読む唯一の関数。値がちょうど `"1"` のときだけ dev。
///
/// `true` / `0` / 空 / 欄なしは dev でない。同名の欄が 2 つ以上あるときも dev でない。
/// **認証を通した後にだけ呼ぶこと** — 公開 route でこの欄を信じてはいけない。
pub fn device_dev_from_headers(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(DEVICE_DEV_HEADER).iter();
    matches!((values.next(), values.next()), (Some(v), None) if v.as_bytes() == b"1")
}

/// `X-Device-Role` を読む唯一の関数。値が運行管理者用の鍵の role にバイト列として
/// 完全一致するときだけ true。
///
/// 別の role / 空 / 大文字違い / 前後の空白 / 欄なしは false。同名の欄が 2 つ以上あるときも false。
/// **認証を通した後にだけ呼ぶこと** — 公開 route でこの欄を信じてはいけない。
pub fn device_tenko_manager_from_headers(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(DEVICE_ROLE_HEADER).iter();
    matches!(
        (values.next(), values.next()),
        (Some(v), None) if v.as_bytes() == DEVICE_ROLE_TENKO_MANAGER.as_bytes()
    )
}

#[derive(Debug, Default)]
struct Marks {
    dev: AtomicBool,
    tenko_manager: AtomicBool,
}

/// 要求 1 件ぶんの印の入れ物。初期値は dev でなく、運行管理者の鍵でもない。
///
/// 認証 middleware が、認証を通した後に書く。clone は同じ中身を指す。
#[derive(Clone, Debug, Default)]
pub struct DeviceDevSlot(Arc<Marks>);

impl DeviceDevSlot {
    pub fn set(&self, dev: bool) {
        self.0.dev.store(dev, Ordering::Relaxed);
    }

    pub fn get(&self) -> bool {
        self.0.dev.load(Ordering::Relaxed)
    }

    /// 運行管理者用の鍵の要求か (dev かどうかとは別の軸)。
    pub fn set_tenko_manager(&self, tenko_manager: bool) {
        self.0.tenko_manager.store(tenko_manager, Ordering::Relaxed);
    }

    pub fn is_tenko_manager(&self) -> bool {
        self.0.tenko_manager.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append(DEVICE_DEV_HEADER, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn only_exactly_one_is_dev() {
        assert!(device_dev_from_headers(&headers(&["1"])));
        for v in ["true", "0", "", " 1", "1 ", "11", "yes"] {
            assert!(!device_dev_from_headers(&headers(&[v])), "{v:?}");
        }
        assert!(!device_dev_from_headers(&headers(&[])));
    }

    #[test]
    fn repeated_header_is_not_dev() {
        assert!(!device_dev_from_headers(&headers(&["1", "1"])));
        assert!(!device_dev_from_headers(&headers(&["0", "1"])));
    }

    #[test]
    fn slot_defaults_to_not_dev_and_clones_share() {
        let slot = DeviceDevSlot::default();
        assert!(!slot.get());
        let other = slot.clone();
        other.set(true);
        assert!(slot.get());
        slot.set(false);
        assert!(!other.get());
    }

    fn role_headers(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append(DEVICE_ROLE_HEADER, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn only_exact_role_is_tenko_manager() {
        assert!(device_tenko_manager_from_headers(&role_headers(&[
            "device-tenko-manager"
        ])));
        for v in [
            "device-kiosk",
            "",
            "Device-Tenko-Manager",
            "DEVICE-TENKO-MANAGER",
            " device-tenko-manager",
            "device-tenko-manager ",
            "device-tenko-manager,device-kiosk",
        ] {
            let got = device_tenko_manager_from_headers(&role_headers(&[v]));
            assert!(!got, "{v:?}");
        }
        assert!(!device_tenko_manager_from_headers(&role_headers(&[])));
    }

    #[test]
    fn repeated_role_header_is_not_tenko_manager() {
        for pair in [
            ["device-tenko-manager", "device-tenko-manager"],
            ["device-kiosk", "device-tenko-manager"],
        ] {
            let got = device_tenko_manager_from_headers(&role_headers(&pair));
            assert!(!got, "{pair:?}");
        }
    }

    #[test]
    fn dev_header_does_not_make_tenko_manager() {
        assert!(!device_tenko_manager_from_headers(&headers(&["1"])));
        assert!(!device_dev_from_headers(&role_headers(&[
            "device-tenko-manager"
        ])));
    }

    #[test]
    fn slot_tenko_manager_defaults_to_false_and_clones_share() {
        let slot = DeviceDevSlot::default();
        assert!(!slot.is_tenko_manager());
        let other = slot.clone();
        other.set_tenko_manager(true);
        assert!(slot.is_tenko_manager());
        // 2 つの印は別の軸 — 片方を書いてももう片方は動かない
        assert!(!slot.get());
        slot.set(true);
        slot.set_tenko_manager(false);
        assert!(!other.is_tenko_manager());
        assert!(other.get());
    }
}
