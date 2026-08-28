/**
 * `t3 theme` - set the environment's theme. Every client applies it: live
 * when connected, on the next connect otherwise. Each client applies a value
 * once, so a theme the user picks in Settings afterwards sticks until the
 * next `t3 theme set`.
 *
 * Writes `defaultTheme` into the environment's `settings.json`. A running
 * server watches that file and pushes the change, so this works before the
 * first launch and on a live server alike.
 *
 * The edit is deliberately a minimal one on the parsed JSON object rather than
 * a schema round-trip. Settings files outlive the build that reads them, and a
 * provisioning command must not drop keys this version does not recognise.
 */
import {
  DefaultThemePreference,
  EnvironmentThemeFile,
  EnvironmentThemeId,
} from "@t3tools/contracts";
import { fromJsonStringPretty, fromLenientJson } from "@t3tools/shared/schemaJson";
import * as Console from "effect/Console";
import * as Effect from "effect/Effect";
import * as FileSystem from "effect/FileSystem";
import * as Option from "effect/Option";
import * as Path from "effect/Path";
import * as Schema from "effect/Schema";
import { Argument, Command, Flag } from "effect/unstable/cli";
import * as CliError from "effect/unstable/cli/CliError";

import { writeFileStringAtomically } from "../atomicWrite.ts";
import * as ServerConfig from "../config.ts";
import { resolveBaseDir } from "../os-jank.ts";
import { baseDirFlag } from "./config.ts";

/** Settings files outlive the build that reads them, so the object is carried
 * as-is and only the one key is touched. */
const SparseSettings = Schema.Record(Schema.String, Schema.Unknown);
const decodeSettingsJson = Schema.decodeUnknownEffect(fromLenientJson(SparseSettings));
const encodeSettingsJson = Schema.encodeEffect(fromJsonStringPretty(SparseSettings));
const decodeThemeId = Schema.decodeUnknownEffect(DefaultThemePreference);
const decodeThemeFileJsonExit = Schema.decodeUnknownExit(
  Schema.fromJsonString(EnvironmentThemeFile),
);
const isEnvironmentThemeId = Schema.is(EnvironmentThemeId);

class ThemeSettingsError extends CliError.UserError {
  override get message() {
    return String(this.cause);
  }
}

const resolveThemePaths = Effect.fn(function* (explicitBaseDir: Option.Option<string>) {
  const baseDir = yield* resolveBaseDir(Option.getOrUndefined(explicitBaseDir));
  const derivedPaths = yield* ServerConfig.deriveServerPaths(baseDir, undefined, {
    baseDirIsExplicit: Option.isSome(explicitBaseDir),
  });
  return {
    settingsPath: derivedPaths.settingsPath,
    themesDir: derivedPaths.environmentThemesDir,
  };
});

/** Reads the sparse settings object, tolerating a missing or empty file. */
const readSettingsObject = Effect.fn(function* (settingsPath: string) {
  const fs = yield* FileSystem.FileSystem;
  const raw = yield* fs.readFileString(settingsPath).pipe(Effect.orElseSucceed(() => ""));
  if (raw.trim().length === 0) return {};

  return yield* decodeSettingsJson(raw).pipe(
    Effect.mapError(
      () =>
        new ThemeSettingsError({
          cause: `${settingsPath} is not a JSON object. Fix or remove it, then run this again.`,
        }),
    ),
  );
});

const writeDefaultTheme = Effect.fn(function* (input: {
  readonly explicitBaseDir: Option.Option<string>;
  readonly themeId: string;
}) {
  const { settingsPath } = yield* resolveThemePaths(input.explicitBaseDir);
  const settings = yield* readSettingsObject(settingsPath);
  const next =
    input.themeId.length > 0
      ? { ...settings, defaultTheme: input.themeId }
      : // Clearing removes the key rather than storing an empty string, so the
        // file reads the same as one that never set a default.
        Object.fromEntries(Object.entries(settings).filter(([key]) => key !== "defaultTheme"));

  const contents = yield* encodeSettingsJson(next);
  yield* writeFileStringAtomically({ filePath: settingsPath, contents: `${contents}\n` });
  return settingsPath;
});

/** Publishes a theme file into the environment's themes directory and returns
 * the id it published under. */
const publishThemeFile = Effect.fn(function* (input: {
  readonly explicitBaseDir: Option.Option<string>;
  readonly filePath: string;
  readonly explicitId: Option.Option<string>;
}) {
  const fs = yield* FileSystem.FileSystem;
  const path = yield* Path.Path;
  const raw = yield* fs
    .readFileString(input.filePath)
    .pipe(
      Effect.mapError(() => new ThemeSettingsError({ cause: `Could not read ${input.filePath}.` })),
    );

  const decoded = decodeThemeFileJsonExit(raw);
  if (decoded._tag === "Failure") {
    return yield* Effect.fail(
      new ThemeSettingsError({
        cause: `${input.filePath} is not a valid theme file. Use a theme exported from T3 Code, or a seeded file with name, appearance, canvas, and accent.`,
      }),
    );
  }
  const file = decoded.value;
  const hasSeeds = file.canvas !== undefined && file.accent !== undefined;
  const hasColors = file.colors !== undefined && Object.keys(file.colors).length > 0;
  if (!hasSeeds && !hasColors) {
    return yield* Effect.fail(
      new ThemeSettingsError({ cause: `${input.filePath} has no colors to publish.` }),
    );
  }

  const fileBasename = path.basename(input.filePath, ".json");
  const themeId = Option.getOrElse(input.explicitId, () => fileBasename);
  if (!isEnvironmentThemeId(themeId)) {
    return yield* Effect.fail(
      new ThemeSettingsError({
        cause: `"${themeId}" is not a valid theme id (lowercase letters, digits, and hyphens). Pass one with --id.`,
      }),
    );
  }

  const { themesDir } = yield* resolveThemePaths(input.explicitBaseDir);
  yield* writeFileStringAtomically({
    filePath: path.join(themesDir, `${themeId}${".json"}`),
    contents: raw.endsWith("\n") ? raw : `${raw}\n`,
  }).pipe(
    Effect.mapError(
      () => new ThemeSettingsError({ cause: `Could not publish the theme into ${themesDir}.` }),
    ),
  );
  return themeId;
});

const themeSetCommand = Command.make("set", {
  baseDir: baseDirFlag,
  id: Flag.string("id").pipe(
    Flag.withDescription("Theme id to publish a file under, instead of its filename."),
    Flag.optional,
  ),
  theme: Argument.string("theme").pipe(
    Argument.withDescription(
      'A theme id (a built-in, or one this machine publishes — themes/omarchy.json is "omarchy"), or a path to a theme JSON file to publish and set in one step.',
    ),
  ),
}).pipe(
  Command.withDescription("Set the environment's theme; connected clients switch to it."),
  Command.withHandler((flags) =>
    Effect.gen(function* () {
      const target = flags.theme.trim();
      if (target.length === 0) {
        return yield* Effect.fail(
          new ThemeSettingsError({
            cause: "Provide a theme id or file, or run `t3 theme clear` to remove the default.",
          }),
        );
      }

      // A path publishes the file first; an id just points at what exists.
      const isFileTarget =
        target.endsWith(".json") || target.includes("/") || target.startsWith("~");
      const themeId = isFileTarget
        ? yield* publishThemeFile({
            explicitBaseDir: flags.baseDir,
            filePath: target,
            explicitId: flags.id,
          })
        : yield* decodeThemeId(target);

      const settingsPath = yield* writeDefaultTheme({
        explicitBaseDir: flags.baseDir,
        themeId,
      });
      yield* Console.log(
        isFileTarget
          ? `Published ${target} as "${themeId}" and set it as the environment theme.\n`
          : `Environment theme set to "${themeId}" in ${settingsPath}.\n`,
      );
    }),
  ),
);

const themeClearCommand = Command.make("clear", { baseDir: baseDirFlag }).pipe(
  Command.withDescription("Remove the environment's theme; clients keep what they have."),
  Command.withHandler((flags) =>
    Effect.gen(function* () {
      const settingsPath = yield* writeDefaultTheme({
        explicitBaseDir: flags.baseDir,
        themeId: "",
      });
      yield* Console.log(`Environment theme cleared in ${settingsPath}.\n`);
    }),
  ),
);

export const themeCommand = Command.make("theme").pipe(
  Command.withDescription("Inspect and set environment-wide theme defaults."),
  Command.withSubcommands([themeSetCommand, themeClearCommand]),
);
