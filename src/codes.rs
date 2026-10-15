//! Diagnostic codes of rusty-cc (prefix C). The codes are fixed identifiers; each one is chosen at an explicit
//! match arm below. Clang makes its diagnostics from text we only see afterwards, so the arms key on the
//! -W option when there is one (stable across wording changes) and otherwise on the start of clang's own message
//! template. Anything not listed gets the generic code of its severity.
use crate::diag::Sev;

pub struct Code { pub code: &'static str, pub symbol_from_quotes: bool }

const fn c(code: &'static str) -> Code { Code { code, symbol_from_quotes: false } }
const fn sym(code: &'static str) -> Code { Code { code, symbol_from_quotes: true } }

pub fn classify(sev: Sev, flag: Option<&str>, msg: &str, driver: bool) -> Code {
    if let Some(f) = flag {
        match f {
            "-Wimplicit-function-declaration" | "-Wimplicit-int" => return sym("C0004"),
            "-Wint-conversion" | "-Wincompatible-pointer-types" | "-Wincompatible-pointer-types-discards-qualifiers"
            | "-Wincompatible-function-pointer-types" | "-Wpointer-sign" | "-Wint-to-pointer-cast" | "-Wpointer-to-int-cast"
            | "-Wconversion" | "-Wshorten-64-to-32" | "-Wimplicit-int-conversion" | "-Wsign-conversion" => return c("C0005"),
            "-Wunused-variable" | "-Wunused-parameter" | "-Wunused-function" | "-Wunused-but-set-variable"
            | "-Wunused-local-typedef" | "-Wunused-value" | "-Wunused-label" | "-Wunused-private-field" => return sym("C0006"),
            "-Wformat" | "-Wformat-security" | "-Wformat-nonliteral" | "-Wformat-extra-args" | "-Wformat-insufficient-args"
            | "-Wformat-invalid-specifier" | "-Wformat-pedantic" | "-Wformat-zero-length" => return c("C0007"),
            "-Wsign-compare" | "-Wtautological-compare" | "-Wtype-limits" => return c("C0010"),
            "-Wdeprecated" | "-Wdeprecated-declarations" | "-Wdeprecated-copy" | "-Wdeprecated-non-prototype" => return sym("C0013"),
            "-Wuninitialized" | "-Wsometimes-uninitialized" | "-Wconditional-uninitialized" => return sym("C0014"),
            "-Wreturn-type" | "-Wreturn-stack-address" => return c("C0016"),
            "-Wunused-command-line-argument" | "-Woverriding-option" | "-Wunknown-warning-option" => return c("C0011"),
            _ => {}
        }
    }
    if driver {
        if msg.starts_with("linker command failed") { return c("C0009"); }
        if msg.starts_with("unknown argument") || msg.starts_with("unsupported option") || msg.starts_with("unknown ") { return c("C0011"); }
        if msg.starts_with("no such file or directory") || msg.starts_with("no input files") { return c("C0012"); }
    } else {
        if msg.starts_with("use of undeclared identifier") || msg.starts_with("use of undeclared label") { return sym("C0001"); }
        if msg.starts_with("expected ") || msg.starts_with("extraneous closing brace") || msg.starts_with("unterminated ") { return c("C0002"); }
        if msg.ends_with("file not found") || msg.contains("' file not found") { return c("C0003"); }
        if msg.starts_with("no matching function") || msg.starts_with("no matching member function") || msg.starts_with("no matching constructor")
            || msg.starts_with("no viable") || msg.starts_with("call to ") && msg.contains("is ambiguous") { return c("C0008"); }
        if msg.starts_with("no member named") || msg.starts_with("no type named") || msg.starts_with("unknown type name")
            || msg.starts_with("no template named") { return sym("C0015"); }
        if msg.starts_with("redefinition of") || msg.starts_with("conflicting types") || msg.starts_with("redeclaration of") { return sym("C0017"); }
        if msg.starts_with("static assertion failed") || msg.starts_with("static_assert") { return c("C0018"); }
    }
    match sev {
        Sev::Fatal => c("C0300"),
        Sev::Error => c("C0100"),
        _ => c("C0200"),
    }
}
