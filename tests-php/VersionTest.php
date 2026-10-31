<?php

declare(strict_types=1);

use Fairlead\Composer\LauncherError;
use Fairlead\Composer\Version;

test('the installed release version is used, with or without its v', function (): void {
    eq('0.14.0', Version::resolve(null, 'v0.14.0', '0.13.0'));
    eq('0.14.0', Version::resolve(null, '0.14.0', '0.13.0'));
    eq('1.0.0-rc.1', Version::resolve(null, 'v1.0.0-rc.1', '0.13.0'));
});

test('FAIRLEAD_VERSION wins over the installed version and the stamped one', function (): void {
    eq('0.13.0', Version::resolve('0.13.0', 'v0.14.0', '0.14.0'));
    eq('0.13.0', Version::resolve('v0.13.0', null, '0.14.0'));
    eq('0.14.0', Version::resolve('', 'v0.14.0', '0.13.0'), 'an empty FAIRLEAD_VERSION is unset');
});

test('the stamped version is the fallback when Composer names none', function (): void {
    eq('0.13.0', Version::resolve(null, null, '0.13.0'));
    check(preg_match('/^\d+\.\d+\.\d+$/', Version::STAMPED) === 1, 'STAMPED is a plain release version');
});

test('a branch install is refused unless FAIRLEAD_VERSION names a release', function (): void {
    foreach (['dev-main', 'dev-feat/x', '1.x-dev', '0.14.x-dev'] as $branch) {
        $message = throws(LauncherError::class, fn () => Version::resolve(null, $branch, '0.14.0'));
        contains("at {$branch}, a branch", $message);
        contains('FAIRLEAD_VERSION=0.14.0', $message);
    }
    eq('0.13.0', Version::resolve('0.13.0', 'dev-main', '0.14.0'));
});

test('anything but a strict release version is refused', function (): void {
    $bad = ['0.14', '0.14.0.0', '01.2.3', '0.14.0+build.1', '../0.14.0', '0.14.0/x', "0.14.0\n", 'latest', 'vv0.14.0', '0.14.0-', '0.14.0-01'];
    foreach ($bad as $version) {
        $message = throws(LauncherError::class, fn () => Version::resolve($version, null, '0.13.0'));
        contains("isn't a release version", $message);
        throws(LauncherError::class, fn () => Version::resolve(null, $version, '0.13.0'));
    }
});
