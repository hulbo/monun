//! SQL's colours, as code editors show them: keywords and types, the functions called, strings
//! (`'…'`, `"…"`, `` `…` ``, PostgreSQL's `$$…$$`), numbers, comments (`--`, `/* */`) and bind
//! parameters (`:name`, `@name`, MyBatis's `#{…}` and `${…}`). The words of Oracle, PostgreSQL,
//! MySQL and SQL Server all count; case doesn't. A word after a dot (`t.date`) is a column.

use std::ops::Range;

use super::Token;

/// Keywords and type names, upper case and sorted (a test checks), looked up by binary search.
const KEYWORDS: &[&str] = &[
    "ADD",
    "AFTER",
    "ALL",
    "ALTER",
    "ALWAYS",
    "ANALYZE",
    "AND",
    "ANY",
    "ARRAY",
    "AS",
    "ASC",
    "AUTHID",
    "AUTO_INCREMENT",
    "BEFORE",
    "BEGIN",
    "BETWEEN",
    "BIGINT",
    "BINARY",
    "BIT",
    "BLOB",
    "BODY",
    "BOOLEAN",
    "BOTH",
    "BULK",
    "BY",
    "BYTEA",
    "CACHE",
    "CALL",
    "CASCADE",
    "CASE",
    "CHAR",
    "CHARACTER",
    "CHARSET",
    "CHECK",
    "CLOB",
    "CLOSE",
    "CLUSTER",
    "COLLATE",
    "COLUMN",
    "COMMENT",
    "COMMIT",
    "CONFLICT",
    "CONNECT",
    "CONSTRAINT",
    "CONTINUE",
    "CREATE",
    "CROSS",
    "CUBE",
    "CURRENT",
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_TIMESTAMP",
    "CURSOR",
    "CYCLE",
    "DATABASE",
    "DATE",
    "DATETIME",
    "DAY",
    "DEC",
    "DECIMAL",
    "DECLARE",
    "DEFAULT",
    "DEFERRABLE",
    "DEFINER",
    "DELETE",
    "DESC",
    "DESCRIBE",
    "DETERMINISTIC",
    "DISABLE",
    "DISTINCT",
    "DO",
    "DOUBLE",
    "DROP",
    "DUPLICATE",
    "EACH",
    "ELSE",
    "ELSIF",
    "ENABLE",
    "END",
    "ENGINE",
    "ESCAPE",
    "EXCEPT",
    "EXCEPTION",
    "EXEC",
    "EXECUTE",
    "EXISTS",
    "EXIT",
    "EXPLAIN",
    "FALSE",
    "FETCH",
    "FIRST",
    "FLOAT",
    "FOLLOWING",
    "FOR",
    "FORALL",
    "FOREIGN",
    "FROM",
    "FULL",
    "FUNCTION",
    "GENERATED",
    "GLOBAL",
    "GO",
    "GOTO",
    "GRANT",
    "GROUP",
    "GROUPING",
    "HAVING",
    "HOUR",
    "IDENTITY",
    "IF",
    "IGNORE",
    "ILIKE",
    "IMMEDIATE",
    "IN",
    "INCREMENT",
    "INDEX",
    "INNER",
    "INOUT",
    "INSERT",
    "INSTEAD",
    "INT",
    "INTEGER",
    "INTERSECT",
    "INTERVAL",
    "INTO",
    "IS",
    "ISOLATION",
    "JOIN",
    "JSON",
    "JSONB",
    "KEY",
    "LANGUAGE",
    "LAST",
    "LATERAL",
    "LEADING",
    "LEFT",
    "LEVEL",
    "LIKE",
    "LIMIT",
    "LOCAL",
    "LOCK",
    "LONG",
    "LONGTEXT",
    "LOOP",
    "MATCHED",
    "MATERIALIZED",
    "MAXVALUE",
    "MEDIUMINT",
    "MEDIUMTEXT",
    "MERGE",
    "MINUS",
    "MINUTE",
    "MINVALUE",
    "MODIFY",
    "MONEY",
    "MONTH",
    "NATURAL",
    "NCHAR",
    "NCLOB",
    "NEXT",
    "NO",
    "NOCACHE",
    "NOCYCLE",
    "NOLOCK",
    "NOT",
    "NOTHING",
    "NOWAIT",
    "NTEXT",
    "NULL",
    "NULLS",
    "NUMBER",
    "NUMERIC",
    "NVARCHAR",
    "NVARCHAR2",
    "OF",
    "OFF",
    "OFFSET",
    "ON",
    "ONLY",
    "OPEN",
    "OPTION",
    "OR",
    "ORDER",
    "OTHERS",
    "OUT",
    "OUTER",
    "OVER",
    "OWNER",
    "PACKAGE",
    "PARTITION",
    "PERFORM",
    "PIVOT",
    "PRAGMA",
    "PRECEDING",
    "PRECISION",
    "PRESERVE",
    "PRIMARY",
    "PRINT",
    "PRIOR",
    "PRIVILEGES",
    "PROCEDURE",
    "PUBLIC",
    "RAISE",
    "RANGE",
    "RAW",
    "READ",
    "REAL",
    "RECORD",
    "RECURSIVE",
    "REFERENCES",
    "REFERENCING",
    "REGEXP",
    "RENAME",
    "REPLACE",
    "RESTRICT",
    "RETURN",
    "RETURNING",
    "RETURNS",
    "REVOKE",
    "RIGHT",
    "RLIKE",
    "ROLE",
    "ROLLBACK",
    "ROLLUP",
    "ROW",
    "ROWID",
    "ROWNUM",
    "ROWS",
    "ROWTYPE",
    "SAVEPOINT",
    "SCHEMA",
    "SECOND",
    "SECURITY",
    "SELECT",
    "SEQUENCE",
    "SERIAL",
    "SESSION",
    "SET",
    "SETS",
    "SHOW",
    "SIBLINGS",
    "SIMILAR",
    "SMALLINT",
    "SOME",
    "START",
    "SYNONYM",
    "SYSDATE",
    "SYSTIMESTAMP",
    "TABLE",
    "TABLESPACE",
    "TEMP",
    "TEMPORARY",
    "TEXT",
    "THEN",
    "TIES",
    "TIME",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "TINYINT",
    "TO",
    "TOP",
    "TRAILING",
    "TRANSACTION",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "TYPE",
    "UNBOUNDED",
    "UNION",
    "UNIQUE",
    "UNIQUEIDENTIFIER",
    "UNKNOWN",
    "UNPIVOT",
    "UNSIGNED",
    "UPDATE",
    "USE",
    "USING",
    "UUID",
    "VALUE",
    "VALUES",
    "VARBINARY",
    "VARCHAR",
    "VARCHAR2",
    "VARYING",
    "VIEW",
    "VOLATILE",
    "WHEN",
    "WHERE",
    "WHILE",
    "WINDOW",
    "WITH",
    "WITHIN",
    "WITHOUT",
    "WORK",
    "WRITE",
    "XML",
    "YEAR",
    "ZONE",
];

/// Functions, upper case and sorted: coloured where they are called (the name followed by `(`),
/// so a column called `count` stays plain.
const FUNCTIONS: &[&str] = &[
    "ABS",
    "ADD_MONTHS",
    "ARRAY_AGG",
    "ASCII",
    "AVG",
    "CAST",
    "CEIL",
    "CEILING",
    "CHARINDEX",
    "CHAR_LENGTH",
    "CHR",
    "COALESCE",
    "CONCAT",
    "CONCAT_WS",
    "CONVERT",
    "COUNT",
    "CUME_DIST",
    "DATEADD",
    "DATEDIFF",
    "DATE_ADD",
    "DATE_FORMAT",
    "DATE_PART",
    "DATE_SUB",
    "DATE_TRUNC",
    "DAY",
    "DECODE",
    "DENSE_RANK",
    "EXTRACT",
    "FIRST_VALUE",
    "FLOOR",
    "FORMAT",
    "GETDATE",
    "GREATEST",
    "GROUP_CONCAT",
    "HOUR",
    "IFNULL",
    "IIF",
    "INITCAP",
    "INSTR",
    "ISNULL",
    "JSON_EXTRACT",
    "JSON_VALUE",
    "LAG",
    "LAST_DAY",
    "LAST_VALUE",
    "LEAD",
    "LEAST",
    "LEFT",
    "LEN",
    "LENGTH",
    "LISTAGG",
    "LN",
    "LOG",
    "LOWER",
    "LPAD",
    "LTRIM",
    "MAX",
    "MEDIAN",
    "MIN",
    "MINUTE",
    "MOD",
    "MONTH",
    "MONTHS_BETWEEN",
    "NEXT_DAY",
    "NOW",
    "NTILE",
    "NULLIF",
    "NVL",
    "NVL2",
    "PERCENT_RANK",
    "POSITION",
    "POWER",
    "RAND",
    "RANDOM",
    "RANK",
    "RATIO_TO_REPORT",
    "REGEXP_INSTR",
    "REGEXP_LIKE",
    "REGEXP_REPLACE",
    "REGEXP_SUBSTR",
    "REPLACE",
    "REVERSE",
    "RIGHT",
    "ROUND",
    "ROW_NUMBER",
    "RPAD",
    "RTRIM",
    "SECOND",
    "SIGN",
    "SQRT",
    "STDDEV",
    "STRING_AGG",
    "STR_TO_DATE",
    "SUBSTR",
    "SUBSTRING",
    "SUM",
    "SYS_GUID",
    "TO_CHAR",
    "TO_DATE",
    "TO_NUMBER",
    "TO_TIMESTAMP",
    "TRANSLATE",
    "TRIM",
    "TRUNC",
    "UNNEST",
    "UPPER",
    "VARIANCE",
    "WM_CONCAT",
    "XMLAGG",
    "YEAR",
];

/// Whether `b` continues a word: letters (any script), digits, `_`, and Oracle's `$` and `#`.
fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'#' || b >= 0x80
}

/// The byte after `text[from..]`'s first `close`, or the end of the text when it never closes.
fn past(text: &str, from: usize, close: &str) -> usize {
    text.get(from..).and_then(|rest| rest.find(close)).map_or(text.len(), |at| from + at + close.len())
}

/// The end of the line holding byte `from` (before its line break).
fn line_end(text: &str, from: usize) -> usize {
    text.get(from..).and_then(|rest| rest.find('\n')).map_or(text.len(), |at| from + at)
}

/// The byte after the `}` closing a `#{…}` or `${…}` at `from`, or the end of its line.
fn brace_end(text: &str, from: usize) -> usize {
    let end = line_end(text, from);
    text.get(from..end).and_then(|rest| rest.find('}')).map_or(end, |at| from + at + 1)
}

/// The end of a string opened by `quote` at `start`: a doubled quote is one quote inside it. One
/// left open runs to the end of the text, as editors show it while it is typed.
fn string_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while let Some(&b) = bytes.get(i) {
        if b == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// The delimiter of a PostgreSQL dollar-quoted string opening at `start` (`$$` or `$tag$`).
fn dollar_tag(text: &str, start: usize) -> Option<&str> {
    let rest = text.get(start + 1..)?;
    let close = rest.find('$')?;
    let tag = rest.get(..close)?;
    let ok = tag.bytes().next().is_none_or(|b| b.is_ascii_alphabetic() || b == b'_') && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if ok { text.get(start..start + close + 2) } else { None }
}

/// The end of a number starting at `start`: digits with a point and an exponent, or `0x…`.
fn number_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    if bytes.get(i) == Some(&b'0') && matches!(bytes.get(i + 1), Some(b'x' | b'X')) {
        i += 2;
        while bytes.get(i).is_some_and(u8::is_ascii_hexdigit) {
            i += 1;
        }
        return i;
    }
    while bytes.get(i).is_some_and(|b| b.is_ascii_digit() || *b == b'.') {
        i += 1;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(i + 1), Some(b'+' | b'-')));
        if bytes.get(i + 1 + sign).is_some_and(u8::is_ascii_digit) {
            i += 1 + sign;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
    }
    i
}

/// How a word is coloured: a function where it is called, a keyword, or not at all. `called` says
/// whether `(` follows it.
fn word_token(word: &str, called: bool) -> Option<Token> {
    if word.len() > 24 || !word.is_ascii() {
        return None;
    }
    let upper = word.to_ascii_uppercase();
    if called && FUNCTIONS.binary_search(&upper.as_str()).is_ok() {
        Some(Token::Builtin)
    } else if KEYWORDS.binary_search(&upper.as_str()).is_ok() {
        Some(Token::Keyword)
    } else {
        None
    }
}

/// The coloured spans of SQL `text` (byte ranges, in order, not overlapping).
pub(super) fn sql(text: &str) -> Vec<(Range<usize>, Token)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    // The last byte before `i` that isn't a space: a word after a dot is a column.
    let mut before = 0u8;
    while let Some(&b) = bytes.get(i) {
        let next = bytes.get(i + 1).copied();
        let span = match b {
            b'-' if next == Some(b'-') => Some((line_end(text, i), Token::Comment)),
            b'/' if next == Some(b'*') => Some((past(text, i + 2, "*/"), Token::Comment)),
            b'\'' | b'"' | b'`' => Some((string_end(bytes, i, b), Token::String)),
            b'$' => match dollar_tag(text, i) {
                Some(tag) if next != Some(b'{') => Some((past(text, i + tag.len(), tag), Token::String)),
                // `${name}` (MyBatis) and `$1` (a PostgreSQL parameter).
                _ if next == Some(b'{') => Some((brace_end(text, i), Token::Builtin)),
                _ if next.is_some_and(|n| n.is_ascii_digit()) => Some((number_end(bytes, i + 1), Token::Builtin)),
                _ => None,
            },
            b'#' if next == Some(b'{') => Some((brace_end(text, i), Token::Builtin)),
            // `:name` and `@name` are parameters; `::type` is a cast.
            b':' | b'@' if before != b':' && next.is_some_and(|n| n.is_ascii_alphanumeric() || n == b'_' || n >= 0x80 || n == b'@') => {
                let mut j = i + 1;
                while bytes.get(j).is_some_and(|&c| word_byte(c) || c == b'@') {
                    j += 1;
                }
                Some((j, Token::Builtin))
            }
            // `.5`, but not the `.5` of `t1.5`.
            d if d.is_ascii_digit()
                || (d == b'.'
                    && next.is_some_and(|n| n.is_ascii_digit())
                    && !i.checked_sub(1).and_then(|p| bytes.get(p)).is_some_and(|&p| word_byte(p))) =>
            {
                Some((number_end(bytes, i), Token::Number))
            }
            w if w.is_ascii_alphabetic() || w == b'_' || w >= 0x80 => {
                let mut j = i + 1;
                while bytes.get(j).is_some_and(|&c| word_byte(c)) {
                    j += 1;
                }
                let word = text.get(i..j).unwrap_or_default();
                let called = bytes.get(j..).is_some_and(|rest| rest.iter().find(|c| **c != b' ' && **c != b'\t') == Some(&b'('));
                let token = if before == b'.' { None } else { word_token(word, called) };
                if let Some(token) = token {
                    out.push((i..j, token));
                }
                before = b'a';
                i = j;
                continue;
            }
            _ => None,
        };
        match span {
            Some((end, token)) => {
                let end = end.clamp(i + 1, text.len());
                // Ranges stay on char boundaries: every delimiter above is ASCII.
                if text.is_char_boundary(end) {
                    out.push((i..end, token));
                    before = b'a';
                    i = end;
                } else {
                    i += 1;
                }
            }
            None => {
                if !b.is_ascii_whitespace() {
                    before = b;
                }
                i += 1;
                // Never stop inside a character.
                while !text.is_char_boundary(i) && i < text.len() {
                    i += 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str) -> Vec<(&str, Token)> {
        sql(text).into_iter().map(|(r, t)| (text.get(r).unwrap_or("?"), t)).collect()
    }

    #[test]
    fn the_word_lists_are_sorted_upper_case() {
        for list in [KEYWORDS, FUNCTIONS] {
            assert!(list.windows(2).all(|w| w[0] < w[1]), "sorted, each once");
            assert!(list.iter().all(|w| w.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')));
        }
    }

    #[test]
    fn a_query_is_coloured_as_code_editors_colour_it() {
        let s = spans("select u.name, count(*) as n\nFROM 사용자 u -- 주석\nwhere u.age >= 20.5 and u.memo = 'it''s' /* 끝 */");
        assert_eq!(
            s,
            vec![
                ("select", Token::Keyword),
                ("count", Token::Builtin),
                ("as", Token::Keyword),
                ("FROM", Token::Keyword),
                ("-- 주석", Token::Comment),
                ("where", Token::Keyword),
                ("20.5", Token::Number),
                ("and", Token::Keyword),
                ("'it''s'", Token::String),
                ("/* 끝 */", Token::Comment),
            ]
        );
    }

    #[test]
    fn functions_only_where_called_and_columns_after_a_dot() {
        assert_eq!(spans("max"), vec![]);
        assert_eq!(spans("MAX (x)"), vec![("MAX", Token::Builtin)]);
        assert_eq!(spans("t.date, date"), vec![("date", Token::Keyword)]);
        assert_eq!(spans("LEFT JOIN"), vec![("LEFT", Token::Keyword), ("JOIN", Token::Keyword)]);
        assert_eq!(spans("varchar2(10)"), vec![("varchar2", Token::Keyword), ("10", Token::Number)]);
        assert_eq!(spans("t1 v$session"), vec![], "digits and $ inside a name");
    }

    #[test]
    fn parameters_strings_and_open_ends() {
        assert_eq!(
            spans("id = #{userId} and :name and @p and $1"),
            vec![
                ("#{userId}", Token::Builtin),
                ("and", Token::Keyword),
                (":name", Token::Builtin),
                ("and", Token::Keyword),
                ("@p", Token::Builtin),
                ("and", Token::Keyword),
                ("$1", Token::Builtin),
            ]
        );
        assert_eq!(spans("x::int"), vec![("int", Token::Keyword)], "a cast is no parameter");
        assert_eq!(spans("$$ body 'x' $$ end"), vec![("$$ body 'x' $$", Token::String), ("end", Token::Keyword)]);
        assert_eq!(spans("$f$ a $f$"), vec![("$f$ a $f$", Token::String)]);
        assert_eq!(spans("'open"), vec![("'open", Token::String)]);
        assert_eq!(spans("/* open"), vec![("/* open", Token::Comment)]);
        assert_eq!(spans("#{open\nselect"), vec![("#{open", Token::Builtin), ("select", Token::Keyword)], "a parameter ends with its line");
        assert_eq!(spans("\"Col\" `c`"), vec![("\"Col\"", Token::String), ("`c`", Token::String)]);
        assert_eq!(spans("0x1F 1e10 .5"), vec![("0x1F", Token::Number), ("1e10", Token::Number), (".5", Token::Number)]);
    }

    #[test]
    fn any_text_colours_without_panicking_on_char_boundaries() {
        for text in ["", "-", "$", "$x", "#", ":", "@", "'", "가'나", "--", "/*", "é$é$", "$가$", "a.", ".", "1e", "0x", "@@가"] {
            for (r, _) in sql(text) {
                assert!(text.get(r.clone()).is_some(), "{text:?}: {r:?}");
            }
        }
    }
}
