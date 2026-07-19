export class ScenarioArguments {
  readonly #values = new Map<string, string | true>();

  constructor(arguments_: readonly string[]) {
    for (const argument of arguments_) {
      if (!argument.startsWith("--") || argument === "--") {
        throw new Error(`scenario arguments must use --name or --name=value: ${argument}`);
      }
      const separator = argument.indexOf("=");
      const name = argument.slice(2, separator < 0 ? undefined : separator);
      const value = separator < 0 ? true : argument.slice(separator + 1);
      if (!/^[a-z][a-z0-9-]*$/u.test(name)) {
        throw new Error(`invalid scenario option name: ${argument}`);
      }
      if (this.#values.has(name)) throw new Error(`duplicate scenario option --${name}`);
      this.#values.set(name, value);
    }
  }

  flag(name: string): boolean {
    const value = this.#take(name);
    if (value === undefined) return false;
    if (value !== true) throw new Error(`--${name} does not take a value`);
    return true;
  }

  string(name: string, fallback?: string): string | undefined {
    const value = this.#take(name);
    if (value === undefined) return fallback;
    if (value === true || value.length === 0) throw new Error(`--${name} requires a value`);
    return value;
  }

  choice<const Choice extends string>(
    name: string,
    choices: readonly Choice[],
    fallback: Choice,
  ): Choice {
    const value = this.string(name);
    if (value === undefined) return fallback;
    if (!choices.includes(value as Choice)) {
      throw new Error(`--${name} must be one of ${choices.join(", ")}`);
    }
    return value as Choice;
  }

  number(
    name: string,
    options: {
      readonly fallback?: number;
      readonly minimum?: number;
      readonly maximum?: number;
      readonly integer?: boolean;
    } = {},
  ): number | undefined {
    const source = this.string(name);
    if (source === undefined) return options.fallback;
    const value = Number(source);
    if (
      !Number.isFinite(value) ||
      (options.integer === true && !Number.isInteger(value)) ||
      (options.minimum !== undefined && value < options.minimum) ||
      (options.maximum !== undefined && value > options.maximum)
    ) {
      const range =
        options.minimum === undefined && options.maximum === undefined
          ? ""
          : ` in ${options.minimum ?? "-infinity"}..=${options.maximum ?? "infinity"}`;
      throw new Error(
        `--${name} must be a${options.integer === true ? "n integer" : " finite number"}${range}`,
      );
    }
    return value;
  }

  pair(
    name: string,
    options: {
      readonly fallback?: readonly [number, number];
      readonly integer?: boolean;
      readonly minimum?: number;
      readonly maximum?: number;
      readonly separator?: string;
    } = {},
  ): readonly [number, number] | undefined {
    const source = this.string(name);
    if (source === undefined) return options.fallback;
    const parts = source.split(options.separator ?? ",").map((part) => part.trim());
    if (parts.length !== 2) throw new Error(`--${name} requires two values`);
    const first = Number(parts[0]);
    const second = Number(parts[1]);
    for (const value of [first, second]) {
      if (
        !Number.isFinite(value) ||
        (options.integer === true && !Number.isInteger(value)) ||
        (options.minimum !== undefined && value < options.minimum) ||
        (options.maximum !== undefined && value > options.maximum)
      ) {
        throw new Error(`--${name} contains an invalid value`);
      }
    }
    return [first, second];
  }

  assertEmpty(): void {
    if (this.#values.size === 0) return;
    throw new Error(
      `unknown scenario option${this.#values.size === 1 ? "" : "s"}: ${[...this.#values.keys()]
        .map((name) => `--${name}`)
        .join(", ")}`,
    );
  }

  #take(name: string): string | true | undefined {
    const value = this.#values.get(name);
    this.#values.delete(name);
    return value;
  }
}
