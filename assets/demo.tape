# Herdr Yank demo. Record it with scripts/record-demo.sh, which sets DEMO_HOME, DEMO_CONFIG,
# and DEMO_PATH for an isolated Herdr. The typed hints (r, j, l) follow the output of the demo
# deploy.sh; check them again after a change to it or to hint assignment.
Output "assets/demo.gif"

Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 16
Set Width 1040
Set Height 560
Set Padding 16
Set Theme "Catppuccin Mocha"
Set WindowBar Colorful
Set Framerate 30
Set TypingSpeed 50ms

Hide
Type `exec env -i HOME="$DEMO_HOME" PATH="$DEMO_PATH" XDG_CONFIG_HOME="$DEMO_CONFIG" TERM=xterm-256color COLORTERM=truecolor LANG=en_US.UTF-8 herdr`
Enter
Sleep 4s
Show

Sleep 500ms
Type "./deploy.sh"
Sleep 300ms
Enter
Sleep 2500ms

# Copy one token: the URL.
Type "# yank: prefix y, then type a hint"
Enter
Sleep 800ms
Ctrl+B
Sleep 500ms
Type "y"
Sleep 2200ms
Type "r"
Sleep 900ms
Type "pbpaste"
Sleep 300ms
Enter
Sleep 2000ms

# Copy several tokens: both pod names.
Type "# several: Tab, pick hints, Enter"
Enter
Sleep 800ms
Ctrl+B
Sleep 500ms
Type "y"
Sleep 1200ms
Tab
Sleep 700ms
Type "j"
Sleep 700ms
Type "l"
Sleep 1000ms
Enter
Sleep 900ms
Type "pbpaste"
Sleep 300ms
Enter
Sleep 3500ms
