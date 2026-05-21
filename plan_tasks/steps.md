Phase 0 — Foundation (~3-5 тижнів solo)

  Section 3 Pass 1 — Найдешевші wins (можна перше)

  Крок 6. Frontend README rewrite (R11 / Phase 0 § 3.5). ~30 хв. Acceptance: BracketChain-Frontend/README.md не містить "simulated transaction" або "pre-SDK"
  claims; описує реальне SDK 0.3.0 wiring.

  Крок 7. Запустити pnpm install в indexer і додати npm install @types/crypto-js якщо ще немає (для HMAC). Перевірити що NestJS raw-body middleware можна
  підключити. ~30 хв.

  Крок 8. Створити bracket-chain-indexer/src/webhooks/helius-hmac.guard.ts як NestJS CanActivate. Логіка: читає X-Helius-Signature, обчислює HMAC-SHA256 над raw   body, порівнює через crypto.timingSafeEqual. ~1 година.

  Крок 9. Додати @UseGuards(HeliusHmacGuard) у bracket-chain-indexer/src/webhooks/webhooks.controller.ts + reєструвати bodyParser.raw({ type: 'application/json'   }) у src/main.ts. ~30 хв.

  Крок 10. Прибрати коментар з HELIUS_WEBHOOK_SECRET="" в .env.example; згенерувати production secret (openssl rand -hex 32); закинути в Railway env vars;
  оновити Helius webhook config через dashboard. ~30 хв. Acceptance: curl -X POST без signature → 401; з valid HMAC → 200.

  Крок 11. Створити bracket-chain-indexer/src/tournaments/dto/check-name.dto.ts (валідація organizer + name query params).

  Крок 12. Додати GET /tournaments/check-name?organizer=&name= в tournaments.controller.ts; реалізувати handler в tournaments.service.ts через
  prisma.tournament.findUnique({ where: { organizer_name: { organizer, name } } }). Response: { taken: boolean, address?: string }. ~2 години.

  Крок 13. Smoke-тест endpoint-у локально + проти Railway-deployed instance. Acceptance: кур-запит на free name → { taken: false }, на taken name → { taken:
  true, address: "..." }.

  Крок 14. Frontend follow-up: створити BracketChain-Frontend/features/tournament/create/hooks/useNameCheck.ts з debounce 300ms. Замінити mock в
  DuplicateNameWarning.tsx:30. ~1 година.

  Section 3 Pass 2 — Anchor tests + CU baseline

  Крок 15. Створити bracket-chain-programs/tests/organizer-deposit.test.ts із 3 тестами: (1) organizerDeposit > 0 refund на pre-start cancel, (2) refund
  idempotency через organizer_deposit_refunded flag, (3) deposit excluded з prize-pool basis у final-match. ~4-6 годин.

  Крок 16. Запустити anchor test. Acceptance: всі 5 existing + 3 нові тести green.

  Крок 17. Створити bracket-chain-programs/tests/capacity-128p-deep.test.ts. Setup: 128 keypairs, 128 ATAs, USDC mint, full join cycle, full bracket report до
  final. Розглянути solana-bankrun-mocha замість solana-test-validator — bankrun запускає 128p test за секунди, не хвилини. ~1 день.

  Крок 18. В capacity test вимірювати CU per ix через getTransaction(sig, { maxSupportedTransactionVersion: 0 }).meta.computeUnitsConsumed. Acceptance: кожен ix   < 1_400_000 CU.

  Крок 19. Створити bracket-chain-programs/CU_BUDGET.md. Записати baseline для всіх ix × preset comb-ів. Це стане Phase 1 redeploy regression contract. ~30 хв.

  Section 2 Pass — Codama integration (4 stages)

  Крок 20. Stage 1.A — встановити Codama dev-deps у bracket-chain-programs/: npm install -D @codama/cli @codama/nodes-from-anchor @codama/renderers-js. ~10 хв.

  Крок 21. Stage 1.B — НЕ запускати npx codama init (interactive flow зламається на Windows). Написати codama.json руками. Use scripts поле з all task, два
  renderer targets: ../bracket-chain-sdk/src/generated + ../bracket-chain-indexer/src/generated. ~30 хв.

  Крок 22. Stage 1.C — запустити make codama-generate (target вже існує). Якщо помилка "Received protocol 'd:'" на Windows — перейти в WSL2 з nvm use 20.20.2 і
  run звідти. Acceptance: src/generated/src/generated/{accounts,errors,instructions,pdas,programs,types}/ створені в обох SDK і indexer. ~1 година.

  Крок 23. Stage 1.D — додати src/generated/ в .gitignore НІ, навпаки закоммітити генерований код (per Solana convention) щоб CI gate міг detect drift. Commit.

  Крок 24. Stage 2.A — створити bracket-chain-indexer/src/webhooks/event-types.ts із hand-typed interfaces для всіх 7 events. Це matches BorshCoder output
  shapes (PublicKey, BN, primitives). Codama renderers-js@2.x не emit event decoders, тому Option A (BorshCoder для events + Codama для accounts/instructions)
  це working compromise. ~2 години.

  Крок 25. Stage 2.B — переписати bracket-chain-indexer/src/webhooks/helius-parser.service.ts: 7 handler signatures з Record<string, unknown> → specific event
  interfaces. Dispatch loop через per-case type narrowing. Acceptance: pnpm typecheck clean. ~4 години.

  Крок 26. Stage 2.C — fix bracket-chain-indexer/scripts/test-parser.mjs (Task #20 з Phase 0 doc) — захоплення current post-Phase-2.5/2.6 webhook payload з
  devnet, замінити stale hardcoded payload. ~2 години.

  Крок 27. Створити bracket-chain-indexer/src/webhooks/helius-parser.service.spec.ts (Phase 0 § 3.2) — 1 happy-path + 1 re-delivery test на кожне з 7 events.
  Mock Prisma через jest-mock-extended's DeepMockProxy<PrismaClient>. ~1 день.

  Крок 28. Створити bracket-chain-indexer/src/reconciliation/reconciliation.service.spec.ts — mock ChainReaderService.fetchTournament returning
  status/champion/slot drift cases. Acceptance: pnpm test зелений; runtime <10s. ~4 години.

  Крок 29. Stage 3.A — SDK install Kit deps: pnpm add @solana/kit @solana/spl-token @solana-program/system @solana-program/token @solana-program/compute-budget.   ~10 хв.

  Крок 30. Stage 3.B — Переписати bracket-chain-sdk/src/client.ts: BracketChainClient тепер тримає rpc: Rpc<SolanaRpcApi>, optional rpcSubscriptions, optional
  signer: TransactionSigner, programAddress: Address. ~2 години.

  Крок 31. Stage 3.C — Переписати bracket-chain-sdk/src/types.ts: re-export Codama-generated Tournament, MatchNode, Participant, ProtocolConfig, enums
  (TournamentStatus, MatchStatus, PayoutPreset). Додати WithAddress<T> wrapper. ~1 година.

  Крок 32. Stage 3.D — Переписати кожен method file: createTournament, joinTournament, startTournament (chunked + compute-budget), reportResult
  (remaining_accounts для final-match), cancelTournament (chunked + remaining_accounts). Кожен ~2-4 години → total ~2 дні.

  Крок 33. Stage 3.E — Переписати subscribe.ts на Kit RPC subscriptions (rpcSubscriptions.accountNotifications(addr).subscribe({ abortSignal })). Same public
  API (onError, kind: 'tournament' | 'match'). ~3 години.

  Крок 34. Stage 3.F — Переписати errors.ts — замінити dead AnchorError branch з mapError на cause-chain walker для
  SolanaError<SOLANA_ERROR__INSTRUCTION_ERROR__CUSTOM>. Перевірити що 21 typed BracketChain* error classes збережені. ~2 години.

  Крок 35. Stage 3.G — Cleanup index.ts: видалити stale exports (TournamentStatusKind, PublicKey re-export, getEnumKind); додати re-exports Kit types. Bump
  package.json версія: 0.3.1 → 0.4.0. ~30 хв.

  Крок 36. Stage 4.A — Frontend install: pnpm add @solana/web3-compat в BracketChain-Frontend/. ~10 хв.

  Крок 37. Stage 4.B — Створити bridge в BracketChain-Frontend/lib/sdk.ts: convert useAnchorWallet() output → Kit's TransactionSigner через @solana/web3-compat.   Reuse в усіх SDK-consuming hooks. ~3 години.

  Крок 38. Stage 4.C — Локально pnpm link SDK в frontend (option B з Phase 0 doc) для swift iteration. Smoke test через browser: connect Phantom → /create →
  join → report → cancel. Acceptance: всі toasts fire, no error toasts, indexer SWR works. ~4 години.

  Крок 39. Опублікувати @bracketchain/sdk@0.4.0 на npm: pnpm publish. Frontend bump dep на ^0.4.0. ~30 хв.

  ★ Insight ─────────────────────────────────────
  - Кроки 29-38 — це найризикованіша частина Phase 0. Чотири дні код-рoboти, але paradigm-shift від Anchor до Kit. Виконуй коммітами після кожного file rewrite
  (Кроки 30, 31, 32 на 5 під-комітів, 33, 34, 35). Якщо щось ламається — є дрібні checkpoints, не один великий "rewrote SDK" commit що зачіпає 15 файлів.
  - **CI gate (Стандартний Codama gate per Phase 0 doc) відкладений намірено до "коли побудуємо cross-repo CI workflow". Local-regen discipline означає: перед
  кожним коммітом в bracket-chain-programs/, ти запускаєш make codama-generate і коммітиш regenerated files в SDK + indexer тим самим pull request або bundle.
  Дисципліна > automation коли ти solo.
  - Stage 4 (frontend bridge) можна частково defer якщо тільки прагнеш девнетного demo, але не справжніх користувачів. Frontend може жити на SDK 0.3.1 (npm)
  поки local 0.4.0 не stabilizes. Це бажано робити перед V1.1 program work, бо V1.1 вимагатиме frontend Steam OpenID changes — а ті ламаються легше на
  стабільному SDK.
  ─────────────────────────────────────────────────

  ---
  Phase 1 Pre-flight

  Крок 40. Згенерувати 5 funded keypairs на devnet: sas-issuer, claim-payer, vrf-payer, refund-payer, cleanup-payer. Закидати airdrop на кожен. Зберегти base58
  secrets через solana-keychain MemoryBackend env vars. ~2 години.

  Крок 41. SAS throwaway test — написати окремий script bracket-chain-indexer/scripts/sas-bootstrap.ts що створює SAS Credential PDA (issuer = sas-issuer
  keypair), SAS Schema PDA з game_identity shape ({ game: u8, steam_id_64: u64, identity_bytes: [u8; 32] }). Acceptance: можеш зчитати створені PDAs з explorer.   ~1 день.

  Крок 42. Switchboard On-Demand devnet sample randomness flow smoke — створити test PullFeed з sample OracleJob, request randomness, verify result on-chain. Це   validate що queue approval (Step 3) працює і ти можеш paying for randomness. ~1 день.

  ---
  Phase 1 — V1.1 (Game schema + SAS identity)

  Крок 43. Створити bracket-chain-programs/programs/bracket-chain/src/state/game.rs із SupportedGame enum (Manual=0, Dota2=1, Cs2Faceit=2, Valorant=3, LoL=4) +
  SettlementMode enum (OrganizerOnly=0, PlayerReported=1, Oracle=2). Re-export у state/mod.rs. ~1 година.

  Крок 44. Розширити state/tournament.rs: додати поля game: SupportedGame, settlement_mode: SettlementMode, dispute_window_secs: u32, vrf_randomness_account:
  Pubkey, vrf_commit_slot: u64, seed_revealed: bool. ~2 години.

  Крок 45. Розширити state/participant.rs: identity_hash: [u8; 32], identity_attestation: Pubkey, wins: u8, losses: u8, points_for: u32, points_against: u32. ~1   година.

  Крок 46. Розширити state/protocol_config.rs: sas_credential: Pubkey, sas_schema: Pubkey. ~30 хв.

  Крок 47. Створити instructions/set_sas_config.rs — admin ix що приймає credential + schema PDAs і записує в protocol_config. ~1 година.

  Крок 48. Розширити instructions/create_tournament.rs: приймає game: SupportedGame + settlement_mode: SettlementMode + dispute_window_secs: u32. Reject
  Cs2Faceit/Valorant/LoL з GameNotYetSupported. ~1 година.

  Крок 49. Розширити instructions/join_tournament.rs: якщо tournament.game != Manual, require attestation: AccountInfo в accounts. Validate що attestation owner   == SAS program, credential + schema match protocol_config, nonce == signer (player wallet), expiry < now. Extract identity_bytes, set
  participant.identity_hash. ~3-4 години.

  Крок 50. Додати event_version: u8 як ПЕРШЕ поле в усіх 7 existing #[event] structs. Initial value EVENT_VERSION_V1 = 1. ~30 хв.

  Крок 51. Frontend (parallel from Step 49) — implement Steam OpenID flow в BracketChain-Frontend/features/auth/steam/. User clicks "Link Steam" → redirect to
  Steam OpenID endpoint → callback returns Steam ID 64 → POST to indexer /identity/steam/attest → indexer issues SAS attestation CPI. ~3-4 дні frontend dev.

  Крок 52. Indexer (parallel) — bracket-chain-indexer/src/identity/identity.controller.ts з POST /identity/steam/attest. Use solana-keychain для sas-issuer
  role. Issue SAS attestation via CPI. ~2 дні.

  ★ Insight ─────────────────────────────────────
  - Крок 49 — найвища technical складність у V1.1. SAS attestation account validation вимагає reading структури Attestation account, check що owner == SAS
  program ID, validate всіх 4 referenced PDAs (credential, schema, nonce=player, signer=issuer), expiry. Anchor's #[account(constraint = ...)] не handles цього
  з коробки — треба ручний validate_attestation() helper. Закладай 1 повний день на цю функцію + unit tests.
  - SAS schema design — irreversible choice. Раз створиш schema PDA з { game: u8, steam_id_64: u64, identity_bytes: [u8; 32] } — це frozen. Якщо пізніше захочеш   додати discord_id або щось — це новий schema, нова attestation ceremony, migration. Думай про fields до Step 47.
  ─────────────────────────────────────────────────

  ---
  Phase 1 — V1 Player-reported + VRF (~4-5 тижнів)

  Крок 53. Розширити state/match_node.rs: додати proposal envelope (7 fields: proposal_source: ProposalSource, proposer: Pubkey, proposed_winner: Pubkey,
  proposed_at: i64, claim_deadline: i64, disputed: bool, dispute_reason: u8). Also додати bracket: u8 PDA seed (schema-prep для formats Phase B). ~1 година.

  Крок 54. Створити state/proposal_source.rs із ProposalSource enum (None=0, Player=1, Oracle=2, GameServer=3). ~10 хв.

  Крок 55. Створити instructions/request_seed.rs — calls Switchboard randomness CPI, зберігає randomness_account + commit slot в Tournament. Authority:
  organizer. ~3 години.

  Крок 56. Створити instructions/reveal_seed.rs — permissionless, читає Switchboard RandomnessAccountData, extracts hash → tournament.seed_hash. Gates
  start_tournament на seed_revealed == true для не-OrganizerOnly tournaments. ~3 години.

  Крок 57. Модифікувати instructions/start_tournament.rs: gate на seed_revealed (skip для OrganizerOnly). ~30 хв.

  Крок 58. Створити instructions/propose_result.rs — signer = player_a або player_b; sets proposal_source = Player, proposer = signer, proposed_winner,
  proposed_at = now, claim_deadline = now + dispute_window_secs. Emit ResultProposed event. ~2 години.

  Крок 59. Створити instructions/confirm_result.rs — signer = counterparty; finalizes match (status → Completed, increments wins/losses/points), advances winner   до next match. Emit MatchReported event. ~2 години.

  Крок 60. Створити instructions/dispute_result.rs — signer = counterparty (Player source) АБО arbitrator (Oracle source). Sets disputed = true, dispute_reason.   Emit ResultDisputed event. ~1 година.

  Крок 61. Створити instructions/claim_result.rs — permissionless. Requires now >= claim_deadline && !disputed. Finalizes proposed_winner. Emit MatchReported.
  ~2 години.

  Крок 62. Створити instructions/resolve_dispute.rs — signer = organizer. Overrides winner, finalizes. Emit MatchReported. ~2 години.

  Крок 63. Створити instructions/force_claim_disputed.rs — permissionless after 24h of organizer silence post-dispute. ~2 години.

  Крок 64. Migration ix instructions/migrate_v1_tournament.rs — reallocs Tournament + MatchNode для нових fields. (Devnet тільки — не потрібно для свіжого
  deploy.) ~1 година.

  Крок 65. Comprehensive mocha tests для всіх 9 нових ix + VRF flow. Use mocked Switchboard randomness де можливо (Switchboard SDK has test helpers). ~3-4 дні.

  Крок 66. Indexer additions: parse 4 нові events (ResultProposed, ResultDisputed, ResultClaimed, DisputeResolved). Add Notification table + WebSocket
  subscription endpoint /notifications/subscribe. ~2 дні.

  Крок 67. Indexer cron: auto-claim.cron.ts — scans matches з proposal_source != None && !disputed && now >= claim_deadline, calls claim_result (signed by
  claim-payer через keychain). Emits Notification. ~1 день.

  Крок 68. Indexer cron: vrf-reveal.cron.ts — scans tournaments з vrf_randomness_account != null && !seed_revealed && now > commit_slot + N, calls reveal_seed.
  Signed by vrf-payer. ~1 день.

  Крок 69. Frontend ReportResultModal rewrite — action-dispatcher routes propose/confirm/dispute/claim/resolve panels базуючись на viewer
  (player_a/player_b/organizer/other) + match state (Active+None / Active+proposed / Disputed / Past-deadline). ~1 тиждень frontend dev.

  ---
  Phase 1 — V1.2 (Switchboard Oracle для Dota 2) (~3-4 тижні)

  Крок 70. Створити state/match_commitment.rs із struct MatchCommitment (lobby_id, player_a_game_id, player_b_game_id, committed_at, committed_slot). Додати
  field commitment: Option<MatchCommitment> в MatchNode. ~30 хв.

  Крок 71. Створити state/oracle_config.rs — extension в protocol_config для Switchboard program ID, queue, staleness threshold, arbitrator pubkey. ~30 хв.

  Крок 72. Створити instructions/set_oracle_config.rs — admin ix. ~30 хв.

  Крок 73. Створити instructions/commit_match_lobby.rs — signer = organizer. Validates match is Active, both players registered with identity_hash (V1.1
  dependency). Writes MatchCommitment. Emit MatchLobbyCommitted event. ~2 години.

  Крок 74. Створити instructions/bind_match_feed.rs — signer = organizer. Validates feed account belongs to Switchboard, OracleJob digest matches expected shape   (params: lobby_id, player_a_game_id, player_b_game_id). Stores feed pubkey on MatchNode. ~3 години.

  Крок 75. Створити instructions/propose_result_oracle.rs — permissionless. Reads bound Switchboard PullFeed, extracts winning game_id, verifies hash matches
  commitment.player_a_game_id OR player_b_game_id. Writes V1's proposal envelope з proposal_source = Oracle. Emit ResultProposed (already exists). ~4 години.

  Крок 76. Модифікувати instructions/dispute_result.rs — broaden signer rule: proposal_source == Oracle → signer ∈ {player_a, player_b, arbitrator}; existing
  Player arm stays. ~30 хв.

  Крок 77. Comprehensive mocha tests — MOCKED feed (не real Switchboard) для всіх V1.2 ix. This is Phase 1 acceptance gate #3. ~3 дні.

  Крок 78. Frontend BindFeedModal + Oracle-pending panel. ~3-4 дні frontend dev.

  Крок 79. Indexer cron: oracle-relayer.cron.ts — scans matches з bound feed, last feed update > N slots, no proposal yet, calls propose_result_oracle. Signed
  by claim-payer. ~1 день.

  ---
  Phase 1 — Program improvements + Partial cancel

  Крок 80. Розширити PayoutPreset enum: add Custom([u16; 8]) variant. Validation rules в create_tournament: sum == 10_000, no gaps, slots[0] > 0,
  placement_count <= max_participants. ~3 години.

  Крок 81. Модифікувати report_result.rs final-match branch: replace hardcoded percentage tables з runtime lookup, handle PayoutPreset::Custom slots. ~2 години.
  Крок 82. Створити instructions/close_tournament.rs — permissionless, callable on status ∈ {Completed, Cancelled, PartialCancelled}. Closes Tournament PDA +
  chunks of MatchNode + Participant PDAs. Rent refunds to original organizer. ~4 години.

  Крок 83. Indexer cron: close-terminal.cron.ts — scans completed tournaments older than 7 days, calls close_tournament chunks. Signed by cleanup-payer. ~1
  день.

  Крок 84. Розширити TournamentStatus enum: PartialCancelled = 5. ~10 хв.

  Крок 85. Створити instructions/partial_cancel_tournament.rs — organizer-signed, callable mid-Active. Flips status. Emit TournamentPartiallyCancelled. ~1
  година.

  Крок 86. Створити instructions/partial_refund_chunk.rs — permissionless, per-call refunds up to ~10 participants based on losses == 0 check. Idempotent via
  refund_paid flag. ~3 години.

  Крок 87. Indexer cron: partial-refund.cron.ts — drives partial refund chunks until done. ~1 день.

  ---
  Phase 1 — Redeploy ceremony

  Крок 88. anchor build → verify clean compile з всіма V1.1+V1+V1.2+improvements+partial-cancel changes. Розглянути CU budget output для regression vs Phase 0 §   3.4 baseline. ~30 хв.

  Крок 89. anchor deploy --provider.cluster devnet → новий program ID. Запис у bracketchain-main/README.md + memory project_state.md. ~30 хв.

  Крок 90. make codama-generate → regenerated SDK + indexer clients автоматично. Verify diff makes sense (нові accounts/instructions/events). ~30 хв.

  Крок 91. SDK bump 0.4.0 → 0.5.0 (Phase 1 major). Update method files для нових ix (всі ~15 нових instructions). ~2-3 дні.

  Крок 92. Indexer Prisma migration: add Participant identity_hash, Match proposal envelope columns, новий Notification table, MatchCommitment columns,
  PartialCancelled enum variant. prisma migrate deploy на Railway. ~4 години.

  Крок 93. Bootstrap ceremony: initialize_protocol на новий program ID, set_sas_config (з credential + schema PDAs from Step 41), set_oracle_config (Switchboard   config). ~30 хв.

  Крок 94. Phase 1 acceptance gates × 9 (з Phase 0 doc § Phase 1 acceptance gates) — full smoke test. Це 2-3 дні перевірок. Acceptance: всі 9 gates green.

  Крок 95. Anchor message — повідомити community channels що Phase 1 live на devnet. ~30 хв.

  ---
  Phase 1.5 — Real Oracle wire-up (~2-3 тижні)

  Крок 96. Специфікувати real OracleJob YAML для Steam Web API GetMatchHistory: parameters lobby_id, player_a_steam_id, player_b_steam_id; output winning
  steam_id_64 hash. ~1 день.

  Крок 97. Специфікувати OpenDota cross-check job: same input, OpenDota API endpoint, same output format. ~1 день.

  Крок 98. Створити Switchboard On-Demand PullFeed на devnet з обома jobs + min_job_responses = 2. ~2 години + lag на approval.

  Крок 99. TEE secret injection: encrypt Steam Web API key with Switchboard TEE pubkey, register through Switchboard CLI. Acceptance: test feed updates через
  mocked Dota 2 match return correct winner hash. ~1 день.

  Крок 100. Frontend BindFeedModal full implementation: organizer creates feed, binds to match PDA в один UX flow. ~3 дні frontend dev.

  Крок 101. Cost monitoring metric в /health — Switchboard feed creation cost + per-update cost rolling sum (R9 mitigation). ~2 години.

  ---
  Phase 1.6 — Production smoke (~1-2 тижні)

  Крок 102. Recruitsuit 4-8 real Dota 2 players (community channel / Discord). Pre-fund their wallets з devnet USDC. ~3 дні coordination.

  Крок 103. End-to-end test #1: 4-player tournament, OrganizerOnly mode, real Dota 2 lobbies, organizer reports. Verify bracket advancement, final payout.
  Acceptance: gravers receive USDC у своїх wallets. ~1 день.

  Крок 104. End-to-end test #2: 4-player tournament, PlayerReported mode, real Steam OpenID, players propose results один одному. Verify dispute window,
  auto-claim cron, bracket advancement. ~1 день.

  Крок 105. End-to-end test #3: 4-player tournament, Oracle mode, real Switchboard PullFeed against real Dota 2 lobby. Organizer commits lobby_id, players play,   oracle proposes, auto-claim. Acceptance: trustless payout без жодних manual reports. ~2 дні.

  Крок 106. Edge-case smoke: gravець свідомо disputes correct oracle result → organizer resolves. Verify bracket finalizes. ~half day.

  Крок 107. CU regression check: pull getTransaction.meta.computeUnitsConsumed для key ix-ів, compare з Phase 0 § 3.4 CU_BUDGET.md baseline. Acceptance: no ix
  grew >10% CU. Якщо є — investigate перед mainnet. ~half day.

  Крок 108. Squads multisig migration: створити 2-of-3 Squads, передати upgrade authority. ~1 день. Acceptance: upgrade tx requires 2 signatures.

  Крок 109. Mainnet pre-flight: оновити Helius до paid plan, Neon Postgres до paid tier, Railway до paid tier, перевірити domain + SSL для bracketchain.xyz,
  configurar Sentry + uptime monitoring. ~2-3 дні DevOps.

  Крок 110. Mainnet deployment ceremony: anchor deploy --provider.cluster mainnet. Real USDC mint integration. Initialize protocol. SAS Credential + Schema
  creation на mainnet (separate ceremony — mainnet SAS не shares devnet state). ~1 день.

  Крок 111. Mainnet smoke: 1 невеликий $1-entry Dota 2 tournament з 4 real players. Verify everything works on mainnet. ~1 день. Acceptance: перший real-money
  tournament finalized без incidents.