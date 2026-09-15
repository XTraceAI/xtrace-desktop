# Contributor guide acceptance

The root development page links to one setup guide and a separate native-detail
page. Existing CI/native commands remain unchanged; documentation introduces no
new runtime behavior or workflow jobs.

| Case                      | Expected result                                                                                                                                                     |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Setup                     | Frozen install and `pnpm check` succeed using repository-pinned toolchains.                                                                                         |
| Gallery walkthrough       | `VITE_GALLERY=1 pnpm dev` serves `/gallery` with explicitly labeled sample data.                                                                                    |
| Fixture commands          | `cargo xtask --help` lists real commands; fixture validation distinguishes populated cases from unimplemented skeletons.                                            |
| Documentation checks      | Every local guide link resolves; documented pnpm scripts exist; all crate directories appear in architecture; the PR template mentions DCO and acceptance evidence. |
| Contribution entry points | Bug, feature and metric issue forms collect reproducible public requirements; CODEOWNERS identifies the maintainer.                                                 |

Record actual results in the PR. Existing native launch and browser suites remain
required for changes affecting those behaviors; editing prose does not itself
certify an app release or product milestone. The README must distinguish the
headless importer from automatic history indexing in the visible app.
