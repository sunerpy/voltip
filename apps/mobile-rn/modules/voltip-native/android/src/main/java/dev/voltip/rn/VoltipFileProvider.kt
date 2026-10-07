package dev.voltip.rn

import androidx.core.content.FileProvider

/**
 * The module's own provider class for history exports (`VoltipHost.shareFile`): libraries that
 * each declared `androidx.core.content.FileProvider` would collide in the merged manifest.
 */
class VoltipFileProvider : FileProvider()
