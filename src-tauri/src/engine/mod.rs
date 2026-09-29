//! Legado 书源引擎：HTTP 取页、规则解析（CSS / XPath / JSON / Regex / @js）、正文排版、书籍与章节解析。
//! 对应原 Electron 版 src/main/legado/ 与 src/shared/novelText.ts。

pub mod http;
pub mod js;
pub mod legado;
pub mod rules;
pub mod source_repo;
pub mod text;
