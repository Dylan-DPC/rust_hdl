// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this file,
// You can obtain one at http://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2019, Olof Kraigher olof.kraigher@gmail.com

use super::*;

#[test]
fn resolves_type_mark_in_subtype_indications() {
    let mut builder = LibraryBuilder::new();
    let code = builder.code(
        "libname",
        "
package pkg1 is
-- Object declaration
constant const : natural := 0;
constant const2 : missing := 0;

-- File declaration
file fil : std.textio.text;
file fil2 : missing;

-- Alias declaration
alias foo : natural is const;
alias foo2 : missing is const;

-- Array type definiton
type arr_t is array (natural range <>) of natural;
type arr_t2 is array (natural range <>) of missing;

-- Access type definiton
type acc_t is access natural;
type acc_t2 is access missing;

-- Subtype definiton
subtype sub_t is natural range 0 to 1;
subtype sub_t2 is missing range 0 to 1;

-- Record definition
type rec_t is record
 f1 : natural;
 f2 : missing;
end record;

-- Interface file
procedure p1 (fil : std.textio.text);
procedure p2 (fil : missing);

-- Interface object
function f1 (const : natural) return natural;
function f2 (const : missing) return natural;
end package;",
    );

    let expected = (0..9)
        .map(|idx| Diagnostic::error(code.s("missing", 1 + idx), "No declaration of 'missing'"))
        .collect();

    let diagnostics = builder.analyze();
    check_diagnostics(diagnostics, expected);
}

#[test]
fn resolves_return_type() {
    let mut builder = LibraryBuilder::new();
    let code = builder.code(
        "libname",
        "
package pkg is
function f1 (const : natural) return natural;
function f2 (const : natural) return missing;
end package;",
    );

    let diagnostics = builder.analyze();
    check_diagnostics(
        diagnostics,
        vec![Diagnostic::error(
            code.s1("missing"),
            "No declaration of 'missing'",
        )],
    );
}

#[test]
fn resolves_attribute_declaration_type_mark() {
    let mut builder = LibraryBuilder::new();
    let code = builder.code(
        "libname",
        "
package pkg is
attribute attr : string;
attribute attr2 : missing;
end package;",
    );

    let diagnostics = builder.analyze();
    check_diagnostics(
        diagnostics,
        vec![Diagnostic::error(
            code.s1("missing"),
            "No declaration of 'missing'",
        )],
    );
}