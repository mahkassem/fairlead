<?php

declare(strict_types=1);

namespace Fairlead\Composer;

/** Runs the binary with this process's arguments, streams and exit code. */
final class Runner
{
    /** @param list<string> $args */
    public static function run(string $bin, array $args): never
    {
        // Replacing the PHP process leaves signals and the exit status to the binary itself.
        if (DIRECTORY_SEPARATOR === '/' && function_exists('pcntl_exec')) {
            @pcntl_exec($bin, $args);
        }
        exit(self::spawn($bin, $args));
    }

    /** @param list<string> $args */
    public static function spawn(string $bin, array $args): int
    {
        $p = proc_open([$bin, ...$args], [STDIN, STDOUT, STDERR], $pipes, null, null, ['bypass_shell' => true]);
        if (!is_resource($p)) {
            throw new LauncherError("can't run {$bin}");
        }
        $code = proc_close($p);
        return $code >= 0 && $code <= 255 ? $code : 1;
    }

    /**
     * FAIRLEAD_VERIFY_ATTESTATION's check: `gh attestation verify` on the archive, before it is
     * unpacked. Asked for and failing, or asked for without gh, stops the install.
     *
     * @param (callable(list<string>): int)|null $exec runs a command, answers its exit code
     * @return (\Closure(string): void)|null
     */
    public static function attestation(string $setting, ?string $gh, ?callable $exec = null): ?\Closure
    {
        if ($setting === '' || $setting === '0' || strtolower($setting) === 'false') {
            return null;
        }
        $exec ??= static function (array $command): int {
            // gh's report goes to stderr, so the binary's own output stays alone on stdout.
            $p = proc_open($command, [STDIN, STDERR, STDERR], $pipes, null, null, ['bypass_shell' => true]);
            return is_resource($p) ? proc_close($p) : 1;
        };
        return static function (string $archive) use ($gh, $exec): void {
            if ($gh === null) {
                throw new LauncherError(
                    "FAIRLEAD_VERIFY_ATTESTATION asks for the build's attestation to be checked, but gh isn't on the PATH. "
                    . 'Install the GitHub CLI (https://cli.github.com), or unset FAIRLEAD_VERIFY_ATTESTATION. Nothing was installed.'
                );
            }
            if ($exec([$gh, 'attestation', 'verify', $archive, '--repo', 'mahkassem/fairlead']) !== 0) {
                throw new LauncherError(
                    'gh attestation verify failed for ' . basename($archive)
                    . ', so it may not be a build of mahkassem/fairlead. Nothing was installed.'
                );
            }
        };
    }
}
