//! 音。**まだ中身がありません。**
//!
//! ワークスペースの枠だけ取ってあります。中にあるのは `cargo new` の
//! ひな形そのままで、使えるものは 1 つもありません。
//!
//! 名前を先に取ったのは、[`gueiz`](../gueiz/index.html) の窓口や
//! 依存の書き方を後から変えずに済ませるためです。
//! 実装を始めるまで、このクレートには依存しないでください。

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
