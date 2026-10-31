<?php

declare(strict_types=1);

use Fairlead\Composer\Cache;

test('the cache roots go FAIRLEAD_CACHE_DIR alone, else the package, the user cache, then temp', function (): void {
    eq([['path' => '/ci/cache', 'shared' => false]], Cache::roots(['FAIRLEAD_CACHE_DIR' => '/ci/cache', 'HOME' => '/home/u'], '/app/vendor/mahkassem/fairlead', false));

    $roots = Cache::roots(['XDG_CACHE_HOME' => '/xdg', 'HOME' => '/home/u'], '/pkg', false);
    eq(['/pkg/.cache', '/xdg/fairlead'], array_column(array_slice($roots, 0, 2), 'path'));
    $roots = Cache::roots(['HOME' => '/home/u'], '/pkg', false);
    eq('/home/u/.cache/fairlead', $roots[1]['path'], 'XDG_CACHE_HOME defaults to ~/.cache');
    if (function_exists('posix_geteuid')) {
        $temp = end($roots);
        eq(true, $temp['shared']);
        check(str_ends_with($temp['path'], '/fairlead-' . posix_geteuid()), 'the temp root is per user');
    }

    $roots = Cache::roots(['LOCALAPPDATA' => 'C:/Users/u/AppData/Local'], 'C:/app/vendor/mahkassem/fairlead', true);
    eq(['C:/app/vendor/mahkassem/fairlead/.cache', 'C:/Users/u/AppData/Local/fairlead'], array_column(array_slice($roots, 0, 2), 'path'));
});

test('a binary already in any root is found, the first root first', function (): void {
    $a = tmpdir();
    $b = tmpdir();
    $roots = [['path' => $a, 'shared' => false], ['path' => $b, 'shared' => false]];
    eq(null, Cache::find($roots, '0.14.0', 'x86_64-unknown-linux-gnu'));
    mkdir("{$b}/0.14.0/x86_64-unknown-linux-gnu", 0777, true);
    touch("{$b}/0.14.0/x86_64-unknown-linux-gnu/fairlead");
    // A read-only vendor/ warmed elsewhere still serves its binary.
    eq("{$b}/0.14.0/x86_64-unknown-linux-gnu/fairlead", Cache::find($roots, '0.14.0', 'x86_64-unknown-linux-gnu'));
    eq($a, Cache::writable($roots));
});

test('a root that cannot be created is skipped for the next', function (): void {
    $file = tmpdir() . '/a-file';
    touch($file);
    $next = tmpdir();
    eq($next, Cache::writable([['path' => "{$file}/sub", 'shared' => false], ['path' => $next, 'shared' => false]]));
    eq(null, Cache::writable([['path' => "{$file}/sub", 'shared' => false]]));
});

test('a shared temp root others can write to is neither searched nor used', function (): void {
    unix();
    if (!function_exists('posix_geteuid')) {
        throw new Skip('needs the posix extension');
    }
    $shared = tmpdir() . '/fairlead-x';
    $roots = [['path' => $shared, 'shared' => true]];
    eq($shared, Cache::writable($roots), 'created for this user alone');
    eq(0700, fileperms($shared) & 0777);
    mkdir("{$shared}/0.14.0/x86_64-unknown-linux-gnu", 0777, true);
    touch("{$shared}/0.14.0/x86_64-unknown-linux-gnu/fairlead");
    eq("{$shared}/0.14.0/x86_64-unknown-linux-gnu/fairlead", Cache::find($roots, '0.14.0', 'x86_64-unknown-linux-gnu'));
    chmod($shared, 0777);
    eq(null, Cache::find($roots, '0.14.0', 'x86_64-unknown-linux-gnu'));
    eq(null, Cache::writable($roots));
});
