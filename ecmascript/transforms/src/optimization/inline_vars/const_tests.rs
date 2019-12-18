//! Copied from https://github.com/google/closure-compiler/blob/6ca3b62990064488074a1a8931b9e8dc39b148b3/test/com/google/javascript/jscomp/InlineVariablesConstantsTest.java

use super::inline_vars;

fn fold(src: &str, expected: &str) {
    test_transform!(
        ::swc_ecma_parser::Syntax::default(),
        |_| inline_vars(),
        src,
        expected,
        true
    )
}

/// Should not modify expression.
fn fold_same(s: &str) {
    fold(s, s)
}

#[test]
fn test_inline_variables_constants() {
    test("var ABC=2; var x = ABC;", "var x=2");
    test("var AA = 'aa'; AA;", "'aa'");
    test("var A_A=10; A_A + A_A;", "10+10");
    test("var AA=1", "");
    test("var AA; AA=1", "1");
    test("var AA; if (false) AA=1; AA;", "if (false) 1; 1;");
    testSame("var AA; if (false) AA=1; else AA=2; AA;");

    // Make sure that nothing explodes if there are undeclared variables.
    testSame("var x = AA;");

    // Don't inline if it will make the output larger.
    testSame("var AA = '1234567890'; foo(AA); foo(AA); foo(AA);");

    test("var AA = '123456789012345';AA;", "'123456789012345'");
}

#[test]
fn test_no_inline_arrays_or_regexps() {
    testSame("var AA = [10,20]; AA[0]");
    testSame("var AA = [10,20]; AA.push(1); AA[0]");
    testSame("var AA = /x/; AA.test('1')");
    testSame("/** @const */ var aa = /x/; aa.test('1')");
}

#[test]
fn test_inline_conditionally_defined_constant1() {
    // Note that inlining conditionally defined constants can change the
    // run-time behavior of code (e.g. when y is true and x is false in the
    // example below). We inline them anyway because if the code author didn't
    // want one inlined, they could define it as a non-const variable instead.
    test("if (x) var ABC = 2; if (y) f(ABC);", "if (x); if (y) f(2);");
}

#[test]
fn test_inline_conditionally_defined_constant2() {
    test(
        "if (x); else var ABC = 2; if (y) f(ABC);",
        "if (x); else; if (y) f(2);",
    );
}

#[test]
fn test_inline_conditionally_defined_constant3() {
    test(
        "if (x) { var ABC = 2; } if (y) { f(ABC); }",
        "if (x) {} if (y) { f(2); }",
    );
}

#[test]
fn test_inline_defined_constant() {
    test(
        "/**\n"
            + " * @define {string}\n"
            + " */\n"
            + "var aa = '1234567890';\n"
            + "foo(aa); foo(aa); foo(aa);",
        "foo('1234567890');foo('1234567890');foo('1234567890')",
    );

    test(
        "/**\n"
            + " * @define {string}\n"
            + " */\n"
            + "var ABC = '1234567890';\n"
            + "foo(ABC); foo(ABC); foo(ABC);",
        "foo('1234567890');foo('1234567890');foo('1234567890')",
    );
}

#[test]
fn test_inline_variables_constants_with_inline_all_strings_on() {
    inlineAllStrings = true;
    test(
        "var AA = '1234567890'; foo(AA); foo(AA); foo(AA);",
        "foo('1234567890'); foo('1234567890'); foo('1234567890')",
    );
}

#[test]
fn test_no_inline_without_const_declaration() {
    testSame("var abc = 2; var x = abc;");
}

// TODO(nicksantos): enable this again once we allow constant aliasing.
#[test]
#[ignore]
fn test_inline_constant_alias() {
    test(
        "var XXX = new Foo(); var YYY = XXX; bar(YYY)",
        "var XXX = new Foo(); bar(XXX)",
    );
}

#[test]
fn test_no_inline_aliases() {
    testSame("var XXX = new Foo(); var yyy = XXX; bar(yyy)");
    testSame("var xxx = new Foo(); var YYY = xxx; bar(YYY)");
}
