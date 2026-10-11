on run argv
  set volumeName to item 1 of argv

  tell application "Finder"
    tell disk volumeName
      open
      set backgroundFile to file ".background:dmg-background.png"

      tell container window
        set current view to icon view
        set toolbar visible to false
        set statusbar visible to false
        set pathbar visible to false
        set bounds to {120, 100, 840, 560}
      end tell

      tell icon view options of container window
        set arrangement to not arranged
        set icon size to 104
        set text size to 13
        set background picture to backgroundFile
      end tell

      set position of item "Xone.app" of container window to {160, 234}
      set position of item "Applications" of container window to {560, 234}
      set position of item "Read Me First.txt" of container window to {360, 340}

      update without registering applications
      delay 2
      close
    end tell
  end tell
end run
