 Open IMPLEMENTATION_CLAIM_REWARD.md to see what’s already done and what’s still missing.

    Pick the single most important missing feature and implement it.

        If it’s not implemented yet, write the code.

        Also write tests (unit tests or integration tests) that verify the new feature.

        Run the appropriate static analyzers:

            For Elixir/Erlang bits (if any), run Dialyzer to catch type/spec issues.

            For the Rust parts, run cargo build to ensure it compiles, then cargo clippy to clean up any lint errors.

    Verify everything still passes.

        Run the full test suite.

        If Dialyzer or Clippy complain, fix those issues too.

    Update IMPLEMENTATION_CLAIM_REWARD.md.
    
2. Update specs/SPECS.md and docs/DOCS.md specs/* and docs/* relevant documentation immediately. 


        Mark the new feature as “implemented” (add date/test-pass note, etc.).

    Commit and push your changes via Git.

        Commit the code, tests, and the updated IMPLEMENTATION_CLAIM_REWARD.md.

        Push to the repo or create a pull request.