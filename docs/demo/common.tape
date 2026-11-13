# Shared by every demo tape; paths are relative to the repository root.
Set Shell bash
# Catppuccin Mocha, with a mauve cursor to match the prompt.
Set Theme { "name": "Catppuccin Mocha", "background": "#1e1e2e", "foreground": "#cdd6f4", "selection": "#585b70", "cursor": "#cba6f7", "black": "#45475a", "red": "#f38ba8", "green": "#a6e3a1", "yellow": "#f9e2af", "blue": "#89b4fa", "magenta": "#f5c2e7", "cyan": "#94e2d5", "white": "#bac2de", "brightBlack": "#585b70", "brightRed": "#f38ba8", "brightGreen": "#a6e3a1", "brightYellow": "#f9e2af", "brightBlue": "#89b4fa", "brightMagenta": "#f5c2e7", "brightCyan": "#94e2d5", "brightWhite": "#a6adc8" }
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 22
Set LineHeight 1.2
Set Padding 28
Set Margin 24
Set MarginFill "#cba6f7"
Set WindowBar Colorful
Set BorderRadius 12
Set TypingSpeed 45ms
Set CursorBlink false

Hide
# render.sh sets PATH and TZ and builds the demo config in $TWYDO_DEMO_DIR.
Type `export XDG_CONFIG_HOME=$TWYDO_DEMO_DIR/config XDG_STATE_HOME=$TWYDO_DEMO_DIR/state VISUAL='nvim --clean' PS1='\[\e[1;38;2;203;166;247m\]❯\[\e[0m\] ' && clear`
Enter
Show
