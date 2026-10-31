<?php

declare(strict_types=1);

namespace Fairlead\Composer;

final class Version
{
    /** The release this launcher shipped with; scripts/docs_check.py holds it to Cargo.toml. */
    public const STAMPED = '0.14.1';

    // Semantic versioning's own grammar, without build metadata, which no release tag carries.
    // Anchored with \z so the version can go into a URL path as it is.
    private const SEMVER = '/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)'
        . '(-(0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*)?\z/';

    /**
     * The release to run: FAIRLEAD_VERSION, else the version Composer installed, else the
     * stamped one. A branch install has no release build, so it needs FAIRLEAD_VERSION.
     */
    public static function resolve(?string $env, ?string $installed, string $stamped = self::STAMPED): string
    {
        if ($env !== null && $env !== '') {
            return self::release($env)
                ?? throw new LauncherError("FAIRLEAD_VERSION={$env} isn't a release version, such as 0.14.0");
        }
        if ($installed !== null) {
            if (str_starts_with($installed, 'dev-') || str_ends_with($installed, '-dev')) {
                throw new LauncherError(
                    "Composer installed mahkassem/fairlead at {$installed}, a branch, which has no release build.\n"
                    . "Require a release (composer require --dev mahkassem/fairlead:^{$stamped}),\n"
                    . "or set FAIRLEAD_VERSION to the release to run, such as FAIRLEAD_VERSION={$stamped}."
                );
            }
            return self::release($installed)
                ?? throw new LauncherError("Composer installed mahkassem/fairlead at {$installed}, which isn't a release version");
        }
        return $stamped;
    }

    /** The version Composer installed this package at, or null when Composer doesn't say. */
    public static function installed(): ?string
    {
        $versions = 'Composer\\InstalledVersions';
        if (!class_exists($versions) || !$versions::isInstalled('mahkassem/fairlead')) {
            return null;
        }
        return $versions::getPrettyVersion('mahkassem/fairlead');
    }

    private static function release(string $version): ?string
    {
        $bare = str_starts_with($version, 'v') ? substr($version, 1) : $version;
        return preg_match(self::SEMVER, $bare) === 1 ? $bare : null;
    }
}
