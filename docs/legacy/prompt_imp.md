What to do:

    Study all specs and requirements.
    
    study specs/SPECS.md and docs/DOCS.md to learn about mapa-kms specifications and IMPLEMENTATION_STATUS.md to understand plan so far.

        Open IMPLEMENTATION_STATUS.md to see what’s already done and what’s still missing.

    Pick the single most important missing feature and implement it.

        If it’s not implemented yet, write the code.

        Also write tests (unit tests or integration tests) that verify the new feature.

        Run the appropriate static analyzers:

            For the Rust parts, run cargo build to ensure it compiles, then cargo clippy to clean up any lint errors.

    Verify everything still passes.

        Run the full test suite.

        If Dialyzer or Clippy complain, fix those issues too.

    Update IMPLEMENTATION_STATUS.md.

        Mark the new feature as “implemented” (add date/test-pass note, etc.).

    
    *** IMPORTANT ***
    If and only if all tests passed in the current milestone you are working on, update the documentation specs/ and docs/ only if necessary and commit the changes.

    