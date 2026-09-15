# Competitor landscape — blockchain-anchored multi-party "signed record" primitive

Research date: 2026-09-14. Method: web search + direct page fetch only; the repo was not read.
Every claim carries a URL. `UNVERIFIED` = I could not confirm it from a fetched page.

## VERDICT: `PARTIAL COMPETITORS`

Strongest counterexample: **OpenAttestation (Singapore GovTech)** — Merkle-batched, hash-only, per-field selective
disclosure, verifiable from a public root, in government production since ~2018
(https://www.developer.tech.gov.sg/products/categories/blockchain/openattestation/overview). It lacks multi-party
signatures and linked personal histories. On the multi-party side the closest are **Woleet** (Bitcoin-anchored e-signature,
Chainpoint receipts, https://blog.woleet.io/eidas-compliant-e-signature-with-bitcoin/) and **EthSign / cyberSign**
(wallet co-signing with public proof, https://docs.ethsign.xyz/ , https://metalex.substack.com/p/cybersign-easy-legal-agreements-on).
No single product found covers all 7 properties; the un-covered combination is stated in §3.

## 1. Product table

Legend: Y = confirmed from source; N = confirmed absent; ? = UNVERIFIED; "–" = not applicable.

| Product | Alive? (last evidence) | Chain | Batches? | Multi-party sigs in record? | Receipt verifiable w/o node? | Linked history / selective disclosure? | Consumer app? | Gov deployments? | Source |
|---|---|---|---|---|---|---|---|---|---|
| OpenTimestamps | Y (v0.7.2 Dec-2024; calendars live 2026 w/ thousands pending) | Bitcoin | Y (Merkle, calendar servers) | N | Y (.ots proof; needs block headers) | N | N (CLI/web) | N | https://en.wikipedia.org/wiki/OpenTimestamps , https://alice.btc.calendar.opentimestamps.org/ |
| Chainpoint (Tierion) | ? — site live, no dated activity found after 2020–21 SEC settlement | Bitcoin | Y (per-minute Merkle root, hourly aggregate to OP_RETURN) | N | Y (Chainpoint Proof JSON-LD, merkleRoot + block) | N | N | N | https://tierion.com/chainpoint/ , https://www.sec.gov/files/litigation/admin/2020/33-10914.pdf |
| Stampery | DEAD — site HTTP 530 (2026-09-14); team pivoted to Witnet | Bitcoin+Ethereum (BTA) | Y (Merkle, arXiv 1711.04709) | N | Y (proof) | N | N | N | https://arxiv.org/pdf/1711.04709 , https://iq.wiki/wiki/luis-cuende |
| Proof of Existence | ? — site returns title only; no activity found after ~2018 | Bitcoin | ? | N | Y (tx id) | N | N | N | https://proofofexistence.com/ , https://www.crunchbase.com/organization/proof-of-existence |
| Woleet | Y (site © 2024; 3 employees May-2026; last funding 2018) | Bitcoin | Y (claims 1000 files/s) | ? (e-signature product; multi-signer not confirmed) | Y (Chainpoint format) | N | N (API/CLI/web) | N | http://www.woleet.io/EN/ , https://tracxn.com/d/companies/woleet/__W4LFqQXE6PPGohl5yZZVxNWHjVTigNeyoRmZHAwKH-0 |
| OriginStamp | Y (2025 blog; "60M+ proofs since 2013") | Bitcoin, Ethereum | Y | N | Y | N | N | N | https://originstamp.com/blog/reader/blockchain-timestamping-2025-data-integrity/en |
| Zoho Sign blockchain timestamp | Y (Bitcoin via OpenTimestamps; Ethereum path discontinued) | Bitcoin | Y (via OTS) | Y — signed doc hash (multi-signer doc) anchored ? | Y (OTS proof, mapped in-app) | N | N (business SaaS) | N | https://www.zoho.com/sign/features-and-benefits/blockchain-based-timestamping.html |
| DocuSign Ethereum integration | DEAD-ish — launched 2018; CEO 2020: "13x too expensive"; no 2024–25 evidence | Ethereum | N | Y (evidence of DocuSigned agreement written to chain) | ? | N | N | N | https://cointelegraph.com/news/blockchain-tech-13x-too-expensive-to-justify-use-docusign-ceo |
| EthSign | Y (110k MAU 2025; $12.6M raised) | BTC/EVM/TON/Solana; Arweave storage | ? | Y (co-signers, EIP-712) | Y ("verified publicly at zero cost"; open-source tool promised) | N | Partial (web dApp, wallet-visible) | N | https://docs.ethsign.xyz/ , https://www.crunchbase.com/organization/ethsign |
| cyberSign (MetaLeX) | Y (launched Feb-2026) | Ethereum/Base + IPFS | N | Y (each party signs with wallet) | Y (on-chain proof) | N | N (wallet UX) | N | https://metalex.substack.com/p/cybersign-easy-legal-agreements-on |
| EAS (Ethereum Attestation Service) | Y (SDK v2 2025) | Ethereum/L2s | Y (multiTimestamp Merkle root) | Partial (single attester per attestation; ref-links between attestations) | Y (offchain attestation + on-chain timestamp) | Partial (refUID linking; no selective disclosure) | N | N | https://github.com/ethereum-attestation-service/eas-docs-site/blob/main/docs/tutorials/timestamping-attestations.md |
| TrueScreen | Y (App Store/Play; 20k+ pro users) | "blockchain timestamping" + EU qualified TSA | ? | N (single capturer) | Y (forensic report) | N | Y (iOS/Android, consumer photo/video certification) | N | https://truescreen.io/ , https://apps.apple.com/it/app/truescreen-certify-photos/id1556229406 |
| Blocknotary | ? (site live; 2024 review listing) | ? | ? | N | ? | N | Partial (interview/journal apps) | N | https://www.blocknotary.com/ |
| stampd.io | DEAD ("no longer in service", records still verifiable) | Bitcoin | ? | N | Y | N | N | N | https://stampd.io/ |
| Factom | DEAD — Factom Inc. Chapter 11 June-2020 | Factom→Bitcoin anchor | Y | N | Y | N | N | Honduras land pilot stalled 2015 | https://www.coindesk.com/markets/2020/06/19/blockchain-company-factom-inc-files-for-chapter-11-bankruptcy |
| Guardtime KSI (Estonia) | Y — but eIDAS qualification dropped 12-Jun-2025 (search snippet; page 404) | Own hash-calendar (KSI); no public chain | Y (hash tree, hash-only) | N | Y (KSI signature) | N | N | Y — Estonian health, property, business, succession registries, courts, State Gazette | https://interoperable-europe.ec.europa.eu/collection/public-sector-tech-watch/use-national-blockchain-infrastructure-support-e-residency-initiative-estonia |
| Georgia NAPR (Bitfury/Exonum) | Y — "NAPR currently uses" (New America, 2026 fetch); ~1.3M docs | Exonum private → Bitcoin anchor | Y | N | Y (digital certificate) | N | N | Y — land titles, mortgages, notary | https://www.newamerica.org/insights/project-capsule-georgia-land-titling-system/ , https://exonum.com/story-georgia |
| Sweden Lantmäteriet (ChromaWay) | DEAD — testbed 2019, "did not advance" | Private (Chromia) | – | Y (multi-party property sale smart contract, PoC) | – | – | – | Pilot only | https://practiceguides.chambers.com/practice-guides/blockchain-2023/sweden/trends-and-developments |
| Dubai Land Department | Y — Prypco Mint tokenization live May-2025; secondary market Feb-2026 | XRP Ledger | – | – | – | – | Y (investor app) | Y — title deeds tokenized, synced to official records | https://dubailand.gov.ae/en/eservices/real-estate-tokenization/ , https://www.coindesk.com/business/2025/05/26/dubai-unveils-real-estate-tokenization-platform-on-xrp-ledger-amid-usd16b-initiative |
| EBSI / Europeum-EDIC | Y — 20 prod nodes; "Business Launch" 2026; targeting qualified e-ledger | Permissioned EU ledger | – | N | Y (VC verification via DID registry) | Y — W3C VCs, selective disclosure via SD-JWT/BBS | Wallet (EUDI) | Y — diplomas (EBSIv2), accreditation | https://ebsi.eu/about-us , https://www.eqar.eu/qa-results/synergies/european-blockchain-service-infrastructure-ebsi/ |
| OpenAttestation (SG GovTech) | Y — ETSI TR 119 476-1 cites it Aug-2025; npm active | Ethereum (document store) | Y (Merkle root per batch) | N (issuer-signed) | Y (root vs. public document store) | Partial — per-field salted obfuscation (selective disclosure), no linked history | N | Y — OpenCerts, HealthCerts, TradeTrust | https://www.developer.tech.gov.sg/products/categories/blockchain/openattestation/overview , https://www.npmjs.com/package/@govtechsg/open-attestation |
| Blockcerts (MIT/Hyland) | Y-ish — stewarded by Hyland; MIT Media Lab no longer active | Bitcoin/Ethereum | Y (Merkle batch) | N | Y | N | N | Y (MIT diplomas; various universities) | https://www.blockcerts.org/guide/faq.html , https://github.com/blockchain-certificates/cert-issuer |
| India CBSE / Maharashtra LegitDoc | Y (CBSE 2021 portal; Maharashtra Ethereum diplomas) | Permissioned (CBSE) / Ethereum (LegitDoc) | ? | N | Y | N | N | Y | https://www.business-standard.com/article/education/cbse-uses-blockchain-for-class-10-12-board-exam-results-documents-121092201094_1.html |
| Andhra Pradesh Mee Bhoomi | Y — pilot launched 23-Jul-2026, 7 mandals, Hyperledger Fabric | Permissioned | – | N | ? | N | N | Y (pilot) | https://www.cointrust.com/news/indias-andhra-pradesh-launches-blockchain-pilot-for-digital-land-records |
| Perú Compras (LACChain) | Y — 154k purchase orders; runs on LACChain **testnet** (Quorum) | Permissioned | ? | N | Y (QR → PDF) | N | N | Y (procurement) | https://www.gob.pe/institucion/perucompras/noticias/297606-peru-compras-registro-en-blockchain-mas-de-154-mil-ordenes-de-compra |
| ChileCompra (BID pilot) | ? — 2018 pilot; no later evidence | ? | ? | N | ? | N | N | Pilot | https://www.chilecompra.cl/2018/07/chilecompra-inicia-proyecto-piloto-para-el-uso-de-la-herramienta-blockchain-en-compras-publicas/ |
| Colombia ANT (Ripple/Peersyst) | STALLED — first deed Jul-2022, "no further updates" after administration change | XRPL | ? | N | Y (QR) | N | N | Pilot | https://cointelegraph.com/news/ripple-s-plan-to-tokenize-colombian-land-stalls-amid-new-administration |
| Blockchain Federal Argentina (sello de tiempo) | DEGRADED — no block mined for ~1 month (2024 report) | Own permissioned chain | ? | N | Y | N | N | Y (Enacom open data, timestamping) | https://bfa.ar/sello , https://www.criptonoticias.com/comunidad/adopcion/blockchain-federal-argentina-no-mina-bloque-hace-casi-mes/ |
| Brazil birth certificate (Growth Tech/IBM) | ? — 3-day pilot 2019; no later evidence | Hyperledger (IBM) | – | N | ? | N | N | Pilot | https://www.biometricupdate.com/201910/blockchain-and-biometrics-leveraged-for-digital-birth-certificate-in-brazil |
| Brazil Pelotas cartório (Ubitquity) | ? — 2017 pilot; Ubitquity itself alive (NFTitle v4.0 Sep-2024) | Bitcoin (Colored Coins) then NFT | – | N | ? | N | N | Pilot | https://www.coindesk.com/markets/2017/04/05/blockchain-land-registry-tech-gets-test-in-brazil , https://ubitquity.medium.com/ubitquity-llc-announces-the-release-of-nftitle-network-v4-0-with-major-enhancements-d56ab7bb5fa0 |
| Illinois birth registry (Evernym/Sovrin) | DEAD — PoC; "key people moved on" (2018) | Sovrin | – | N | – | – | N | Pilot | https://blockchan.ge/blockchange-birth-registration.pdf |
| Sovrin / Hyperledger Indy | Sovrin DEAD — Foundation dissolved 21-May-2025, ledger read-only archive | Indy | – | N | – | Y (VCs/ZKP) | Wallets | Various | https://sovrin.org/sovrin-foundation-mainnet-ledger-shutdown-likely-on-or-before-march-31-2025/ , https://www.autheo.com/blog/what-happened-to-sovrin-network |
| Microsoft Entra Verified ID | Y — but did:ion removed; did:web only | None now (did:web) | – | N | Y | Selective disclosure "planned" | Authenticator app | Some | https://learn.microsoft.com/en-us/entra/verified-id/whats-new |
| Privado ID (ex-Polygon ID) | Y | Polygon (state roots) | – | N | Y | Y (ZK selective disclosure) | Wallet | N found | https://docs.privado.id/docs/introduction/ |
| World ID | Y — credentials in 13+ countries 2025 | World Chain | – | N | Y | Y (proof of age/nationality, ZK) | Y (World App) | N | https://world.org/blog/announcements/new-world-id-passport-credential-launches-access-wld-tokens |
| Zirtue (US friends/family loans) | PIVOTED — general P2P loans stopped 11-Nov-2025; Bill-Pay only | None | – | Y (both parties agree in-app) | N | N | Y | N | https://intercom.help/zirtue-helpcenter/en/articles/12773144-what-s-changing-with-zirtue-loans |
| Kuen (MX, informal loans) | Y — founded 2025 | None | – | Y ("validación mutua") | N | N | Y | N | https://gerenciaynegocios.com/29-millones-de-mexicanos-solicitan-prestamos-a-conocidos-y-esta-app-busca-formalizarlos/ |
| Smart-IOU (Android) | ? — listing exists; content not fetched | None stated | – | Y (two-party validation) | N | N | Y | N | https://play.google.com/store/apps/details?id=com.techtool.ioucontractvalidator |
| Propy | Y — county-recorded deeds (Vermont 2018, Arizona 2022); $100M title-firm buyout plan 2026 | Ethereum/others | N | Y (buyer/seller/title in smart contract) | ? | N | Y (web/app) | Y (county recorders accept, on-chain is supplementary) | https://www.govtech.com/biz/Vermont-City-Real-Estate-Startup-Try-Out-Blockchain-for-Recording-Property-Transactions.html , https://www.inman.com/2026/05/14/propy-ai-title-companies/ |
| RealT | DEAD — liquidation 2-Jul-2026 | Ethereum/Gnosis | – | – | – | – | Y | N | https://eco.com/support/en/articles/15254024-tokenized-real-estate-2026-realt-lofty-propy-compared |
| Lofty | Y — sole US retail platform; CA DFPI 2022 consent order | Algorand | – | – | – | – | Y | N | https://eco.com/support/en/articles/15254024-tokenized-real-estate-2026-realt-lofty-propy-compared |

Not found at all (searched, no product-level evidence): Adobe/Acrobat Sign blockchain anchoring; Signaturit blockchain
anchoring (they use an EU-accredited TSA, not a chain — https://www.signaturit.com/blog/the-time-stamp-authority-a-seal-to-provide-greater-security-to-electronic-signatures/);
Sign.io; any LATAM consumer IOU app with a public-chain anchor.

## 2. Coverage matrix (7 properties)

| # | Property | Covered by | Gap |
|---|---|---|---|
| 1 | Salted doc hash signed by ALL parties (1..n) | EthSign, cyberSign, DocuSign-2018 (multi-party); OTS/Chainpoint/Woleet/OriginStamp (1-party); Woleet multi-signer UNVERIFIED | Covered |
| 2 | Merkle-batch many leaves → one root per block; root off-state | OpenTimestamps, Chainpoint, Stampery, Woleet, OriginStamp, EAS multiTimestamp, OpenAttestation, Blockcerts, KSI | Covered — this is the 2013–2017 state of the art |
| 3 | Compact receipt (path + block ref), QR-sized, verifiable with a header only | OTS (.ots), Chainpoint Proof, OpenAttestation, Blockcerts, Perú Compras/Colombia (QR→server, weaker) | Covered (QR form factor is packaging, not novelty) |
| 4 | Records LINK to prior records; per-person self-owned history; selective disclosure over the chain of records | EAS refUID (linking, no privacy); OpenAttestation (per-field disclosure, no linking); VC/BBS/SD-JWT/Privado/World ID (selective disclosure of single-issuer credentials, no bilateral co-signed credential standard — https://www.w3.org/TR/vc-data-model-2.0/) | **NOT covered as a combination**: nobody offers a linkable, selectively-disclosable history of *two-party co-signed* records |
| 5 | Hash-only on chain; erasure-compatible | Every anchoring product above; Estonia KSI; Georgia; OpenAttestation | Covered — table stakes |
| 6a | Government civil registries / deeds / procurement | Estonia (KSI), Georgia (Bitcoin-anchored), Dubai (XRPL), Perú Compras, CBSE, Singapore OA, EBSI diplomas | Covered — but each is a bespoke national integration, not a shared public-chain primitive |
| 6b | Ordinary-people use (loans, handover photos, car sales, receipts, authorship) | TrueScreen (photos, 1-party); Kuen/Zirtue/Smart-IOU (loans, no chain); OTS (authorship, techie) | **Partially covered**; no consumer two-party anchored product found |
| 7 | Consumer phone app, chain invisible | TrueScreen (yes, 1-party); Kuen (yes, no chain); EthSign/cyberSign (wallet-visible) | **NOT covered** together with #1 |

**Real differentiator (what nobody covers):** properties **1 + 4 + 7 together** — a consumer app where two ordinary people
co-sign a record, receive a header-verifiable receipt, and the record links into a personal, selectively-disclosable
history. Properties 2, 3, 5 individually are solved and commoditised (OpenTimestamps has done them for free since 2016).
Property 6a is dominated by permissioned/national systems that governments already run.

## 3. Graveyard — dead or stalled attempts and why

| Project | Died/stalled | Cause (sourced) |
|---|---|---|
| Factom Inc. | Chapter 11, Jun-2020 | Could not raise; $18M burned, losses every year 2016–19 — https://www.coindesk.com/markets/2020/06/19/blockchain-company-factom-inc-files-for-chapter-11-bankruptcy |
| Factom × Honduras land registry | Stalled Dec-2015 | "Political issues" / government unwilling — https://www.coindesk.com/markets/2015/12/26/blockchain-land-title-project-stalls-in-honduras |
| Tierion / Chainpoint | SEC settlement Dec-2020, repay $25M TNT ICO; no dated activity since | Token-funded infra hit securities law — https://blockchain.bakermckenzie.com/2021/01/04/sec-settlement-with-tierion-requires-repayment-to-tnt-token-purchasers/ |
| Stampery | Site down (HTTP 530, 2026-09-14); team → Witnet/Aragon | Pivot away from timestamping; $610K raised total — https://www.crunchbase.com/organization/stampery |
| stampd.io | "No longer in service" | Unstated; records remain verifiable — https://stampd.io/ |
| Proof of Existence | No evidence of activity after ~2018 | UNVERIFIED cause; OTS made the same thing free |
| DocuSign Ethereum | Launched 2018, CEO 2020: $1/agreement vs $0.07 = "13x too expensive" | Unit economics of per-record on-chain writes — https://cointelegraph.com/news/blockchain-tech-13x-too-expensive-to-justify-use-docusign-ceo |
| Sweden Lantmäteriet | Testbed 2019, "did not advance" | Never left PoC; needed law changes for digital title transfer — https://practiceguides.chambers.com/practice-guides/blockchain-2023/sweden/trends-and-developments |
| Illinois birth registry | PoC 2017, staff left 2018 | Champion attrition — https://blockchan.ge/blockchange-birth-registration.pdf |
| Colombia ANT × Ripple | First deed Jul-2022, then silence | New administration — https://cointelegraph.com/news/ripple-s-plan-to-tokenize-colombian-land-stalls-amid-new-administration |
| Sovrin Foundation | Dissolved 21-May-2025 | $2M+ debt, steward attrition, no new endorsers in 2024 — https://www.autheo.com/blog/what-happened-to-sovrin-network |
| Microsoft ION (did:ion) | Removed from Entra Verified ID | Replaced by did:web (no chain) — https://learn.microsoft.com/en-us/entra/verified-id/whats-new |
| Blockchain Federal Argentina | ~1 month without a block (2024) | Validator attrition on a permissioned gov chain — https://www.criptonoticias.com/comunidad/adopcion/blockchain-federal-argentina-no-mina-bloque-hace-casi-mes/ |
| RealT | Voluntary liquidation 2-Jul-2026 | Regulatory pressure, US exit 2023, ~$640K escrow vs 14–22k investors — https://eco.com/support/en/articles/15254024-tokenized-real-estate-2026-realt-lofty-propy-compared |
| Zirtue (P2P friend loans) | Stopped general P2P loans 11-Nov-2025 | Pivot to bill-pay lending (monetisable) — https://intercom.help/zirtue-helpcenter/en/articles/12773144-what-s-changing-with-zirtue-loans |
| Guardtime KSI eIDAS | Qualification dropped 12-Jun-2025 (service continues) | Search-snippet only; page 404 — UNVERIFIED — https://grdtm.voog.com/blog/ksi-service-eidas-qualification-discontinuation-notification |

Pattern in the graveyard: (a) consumer/SME timestamping never found a price above "free" once OpenTimestamps existed;
(b) token-funded infra (Tierion, Factom) died of securities law and burn; (c) government pilots die of champion turnover
and elections, not of technology; (d) per-record on-chain writes (DocuSign) fail on unit cost — which is exactly what
batching fixes, and batching was already solved by everyone in (a).

## 4. Regulatory facts (sourced)

- **eIDAS Art. 41 (EU 910/2014)**: any electronic timestamp (blockchain included) "shall not be denied legal effect ... solely"
  for not being qualified; only a *qualified* timestamp from a QTSP gets the presumption of accuracy/integrity —
  https://ec.europa.eu/digital-building-blocks/sites/spaces/DIGITAL/pages/880312429/eSignature+FAQ ,
  https://www.trueoriginal.com/insights/are-blockchain-timestamps-eidas-qualified-timestamps
- **eIDAS 2.0 (EU 2024/1183) Art. 45i**: new "qualified electronic ledger" trust service with a legal presumption of
  chronological ordering and integrity; Commission reference standards due by 21-May-2025; Qualified Ledger category
  open on the Trusted List as of Apr-2026 (b2trust) — https://eur-lex.europa.eu/legal-content/EN/TXT/PDF/?uri=OJ%3AL_202401183 ,
  https://b2trust.com/en/blog/eidas-2-implementation-status-eu-27-q2-2026
- **Italy, L. 12/2019 art. 8-ter c.3**: DLT storage of a document "produces the legal effects of electronic time validation"
  under eIDAS Art. 41; AgID technical guidelines required and, per commentators, not implemented —
  https://natlawreview.com/article/italy-s-legal-recognition-blockchain-based-timestamping
- **US ESIGN Act, 15 U.S.C. §7001**: a signature/record "may not be denied legal effect ... solely because it is in
  electronic form"; UETA adopted by 49 states — https://www.law.cornell.edu/uscode/text/15/7001
- **Vermont 12 V.S.A. §1913 (2016)**: blockchain-registered record is self-authenticating (V.R.E. 902) and a business record
  (803(6)) if accompanied by a sworn declaration — https://legislature.vermont.gov/statutes/section/12/081/01913
- **Arizona A.R.S. §44-7061 (HB 2417, 2017)**: a signature or record "secured through blockchain technology" is an electronic
  signature/record; smart-contract terms enforceable — https://www.azleg.gov/ars/44/07061.htm
- **China**: Hangzhou Internet Court accepted blockchain-preserved evidence (Jun-2018); SPC Internet-Court Provisions
  (7-Sep-2018) recognise blockchain evidence if legitimacy proven; SPC Online Litigation Rules (effective 1-Aug-2021)
  extend review rules to all courts — https://www.loc.gov/item/global-legal-monitor/2018-09-21/china-supreme-court-issues-rules-on-internet-courts-allowing-for-blockchain-evidence/ ,
  https://www.loc.gov/item/global-legal-monitor/2021-07-21/china-supreme-people%27s-court-issues-online-litigation-rules-addressing-review-of-blockchain-evidence/
- **Mexico**: electronic pagarés valid after the 2024 commercial reform if signed with a FEA from an authorised
  certification provider (secondary source) — https://financera.mx/prestamos/pagare/
- Comparative state list (Delaware, Ohio, Illinois) — https://truescreen.io/articles/blockchain-evidence-court-admissibility-standards/

Not asserted: adoption trends, court-case volumes, or any "jurisdiction recognises blockchain timestamps" claim beyond the above.

## 5. Brutally honest summary (10 lines)

1. Merkle-batching document hashes into one on-chain root with a compact inclusion receipt is a 2013–2016 invention (Proof of Existence, Chainpoint, OpenTimestamps); it is free, open, and still running. Properties 2, 3 and 5 are re-inventions.
2. Government hash-only registries exist and run today: Estonia KSI (since 2012), Georgia NAPR on Bitcoin (1.3M docs), Singapore OpenAttestation, Perú Compras, CBSE. Property 6a is not new; the novelty would only be "on a shared public PoS chain", which governments have repeatedly declined (Fabric/Quorum/KSI/Exonum chosen every time).
3. Multi-party wallet signing of an agreement hash with public verification exists: EthSign (110k MAU), cyberSign (2026), DocuSign's 2018 Ethereum path. Property 1 is not new; it is unproven as a *consumer* product.
4. Selective disclosure exists and is standardised (W3C VC 2.0 May-2025, SD-JWT, BBS, Privado, World ID). What does not exist anywhere found: a **bilateral, co-signed credential that links into a personal history** — property 4 is the only genuinely open slot, and it is a data-model problem, not a chain problem.
5. The only consumer notarization app with real traction found is TrueScreen (single-party photo/video, sells to lawyers/insurers, pairs blockchain with an eIDAS *qualified* TSA). Consumers pay for legal weight, not for a hash.
6. Legal weight comes from qualified timestamps (eIDAS Art. 41 presumption) or sworn declarations (Vermont), not from the chain; a DOLI anchor alone gets "not denied admissibility", the weakest tier. Guardtime itself dropped KSI's eIDAS qualification in 2025 (UNVERIFIED page).
7. The graveyard says consumer timestamping has zero willingness-to-pay once a free option exists, token-funded anchoring infra dies of securities law, and government pilots die of elections and champion turnover — none died of missing technology.
8. Friend-loan apps (Zirtue, Kuen) show demand for the *workflow* (reminders, mutual confirmation, payments) and no demand for the *proof*; Zirtue abandoned general P2P loans in Nov-2025 because it could not monetise them.
9. Real-world asset registries on public chains are being cleared out by regulators (RealT liquidated Jul-2026, Lofty barred in CA); Dubai's XRPL programme survives because the *government* is the issuer — the chain is a vendor choice, not a moat.
10. Net: the primitive is a re-implementation of OpenTimestamps + Chainpoint with a co-signature field; the defensible part is the linked, selectively-disclosable two-party history plus a consumer UX that hides the chain — and that part has no proven market yet.
