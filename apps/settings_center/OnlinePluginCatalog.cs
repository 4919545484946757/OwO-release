namespace OwO_Settings;

internal sealed record OnlinePluginCatalogSnapshot(string ReleaseTag, IReadOnlyList<OnlineResource> Resources);

// A settings-window cache. Only a completed, validated fetch replaces it;
// an empty catalog is also a successful result.
internal sealed class OnlinePluginCatalog(OnlineDistribution client)
{
    internal OnlinePluginCatalogSnapshot? Current { get; private set; }

    internal async Task<OnlinePluginCatalogSnapshot> LoadAsync(bool refresh, CancellationToken token)
    {
        if (!refresh && Current is { } cached) return cached;
        var release = await client.LatestAsync(token);
        var resources = await client.ResourcesAsync(release, token);
        token.ThrowIfCancellationRequested();
        return Current = new(release.Tag, resources.Where(resource => resource.Kind == "plugin").ToArray());
    }
}
