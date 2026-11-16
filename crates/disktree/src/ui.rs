//! XAML markup for the window and the table rows (loaded with XamlReader at run time).

pub const NS: &str = r##"xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml""##;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn item(name: &str, text: &str, key: &str, mods: &str) -> String {
    let acc = if key.is_empty() {
        String::new()
    } else if mods.is_empty() {
        format!(r##"<MenuFlyoutItem.KeyboardAccelerators><KeyboardAccelerator Key="{key}"/></MenuFlyoutItem.KeyboardAccelerators>"##)
    } else {
        format!(r##"<MenuFlyoutItem.KeyboardAccelerators><KeyboardAccelerator Key="{key}" Modifiers="{mods}"/></MenuFlyoutItem.KeyboardAccelerators>"##)
    };
    format!(r##"<MenuFlyoutItem x:Name="{name}" Text="{text}">{acc}</MenuFlyoutItem>"##)
}

fn toggle(name: &str, text: &str, checked: bool, key: &str, mods: &str) -> String {
    let acc = if key.is_empty() {
        String::new()
    } else if mods.is_empty() {
        format!(r##"<ToggleMenuFlyoutItem.KeyboardAccelerators><KeyboardAccelerator Key="{key}"/></ToggleMenuFlyoutItem.KeyboardAccelerators>"##)
    } else {
        format!(r##"<ToggleMenuFlyoutItem.KeyboardAccelerators><KeyboardAccelerator Key="{key}" Modifiers="{mods}"/></ToggleMenuFlyoutItem.KeyboardAccelerators>"##)
    };
    format!(r##"<ToggleMenuFlyoutItem x:Name="{name}" Text="{text}" IsChecked="{checked}">{acc}</ToggleMenuFlyoutItem>"##)
}

fn radio(name: &str, text: &str, checked: bool) -> String {
    format!(r##"<RadioMenuFlyoutItem x:Name="{name}" Text="{text}" GroupName="units" IsChecked="{checked}"/>"##)
}

const SEP: &str = "<MenuFlyoutSeparator/>";

/// Glass surfaces over the acrylic window.
const CARD: &str = r##"CornerRadius="8" Background="#12FFFFFF" BorderBrush="#1CFFFFFF" BorderThickness="1""##;
const PANE: &str = r##"CornerRadius="8" Background="#0DFFFFFF" BorderBrush="#18FFFFFF" BorderThickness="1""##;

fn hud() -> String {
    let lanes: String = (0..crate::hud::LANES)
        .map(|k| {
            format!(
                r##"<Grid x:Name="Lane{k}" Height="20" VerticalAlignment="Top" Opacity="0" ColumnSpacing="8">
  <Grid.ColumnDefinitions><ColumnDefinition Width="56"/><ColumnDefinition Width="*"/><ColumnDefinition Width="70"/></Grid.ColumnDefinitions>
  <TextBlock x:Name="Lane{k}Name" FontSize="11.5" VerticalAlignment="Center"/>
  <Border x:Name="Lane{k}Bar" Grid.Column="1" Height="8" CornerRadius="4" HorizontalAlignment="Left" Width="2" Background="#FF4CC2FF"/>
  <TextBlock x:Name="Lane{k}Val" Grid.Column="2" FontSize="11.5" Foreground="#FFA8AFC1" HorizontalAlignment="Right" VerticalAlignment="Center"/>
</Grid>"##
            )
        })
        .collect();
    let stat = |label: &str, name: &str, col: u8, row: u8| {
        format!(
            r##"<StackPanel Grid.Column="{col}" Grid.Row="{row}"><TextBlock Text="{label}" FontSize="10.5" Foreground="#FF7A8296"/><TextBlock x:Name="{name}" Text="0" FontSize="17" FontWeight="SemiBold"/></StackPanel>"##
        )
    };
    format!(
        r##"<Border x:Name="Hud" HorizontalAlignment="Right" VerticalAlignment="Top" Margin="14" Width="336" CornerRadius="10" Padding="14,12" BorderBrush="#26FFFFFF" BorderThickness="1" Visibility="Collapsed" Opacity="0" IsHitTestVisible="False">
  <Border.Background><AcrylicBrush TintColor="#FF141620" TintOpacity="0.55" TintLuminosityOpacity="0.7" FallbackColor="#E8161822"/></Border.Background>
  <StackPanel Spacing="8">
    <Grid>
      <TextBlock x:Name="HudMethod" Text="Reading" FontSize="11" FontWeight="SemiBold" Foreground="#FF7A8296" CharacterSpacing="60"/>
      <TextBlock x:Name="HudPct" Text="0%" FontSize="11" FontWeight="SemiBold" Foreground="#FF4CC2FF" HorizontalAlignment="Right"/>
    </Grid>
    <Image x:Name="HudImage" Height="76" Stretch="Fill"/>
    <Grid ColumnSpacing="14" RowSpacing="6">
      <Grid.ColumnDefinitions><ColumnDefinition/><ColumnDefinition/></Grid.ColumnDefinitions>
      <Grid.RowDefinitions><RowDefinition/><RowDefinition/></Grid.RowDefinitions>
      {s1}{s2}{s3}{s4}
    </Grid>
    <Grid Height="134">{lanes}</Grid>
  </StackPanel>
</Border>"##,
        s1 = stat("Files", "HudFiles", 0, 0),
        s2 = stat("Folders", "HudFolders", 1, 0),
        s3 = stat("Data found", "HudBytes", 0, 1),
        s4 = stat("Items / s", "HudRate", 1, 1),
    )
}

pub fn page(drives: &[String]) -> String {
    let drive_items: String = drives
        .iter()
        .map(|d| format!(r##"<ComboBoxItem Content="{}"/>"##, esc(d)))
        .collect();
    let file_menu = [
        item("mScan", "Scan", "F5", ""),
        item("mSelectFolder", "Select Folder...", "F3", ""),
        item("mSelectMultiple", "Select Multiple Folders...", "F3", "Shift"),
        item("mSelectCsv", "Open CSV File...", "O", "Control"),
        item("mStop", "Stop Scan", "Escape", ""),
        item("mNewInstance", "New Instance", "N", "Control"),
        SEP.to_string(),
        item("mReset", "Reset tree expansion", "F4", ""),
        item("mExpandAll", "Expand All", "F4", "Shift"),
        item("mParent", "Select Parent", "F6", ""),
        item("mReselectChild", "Reselect Child", "F6", "Shift"),
        item("mZoomIn", "Zoom in Treemap", "F10", ""),
        item("mZoomOut", "Zoom out Treemap", "F11", ""),
        item("mSavePng", "Save Treemap Image as .png file", "S", "Control"),
        item("mFilter", "Filter...", "F", "Control,Shift"),
        SEP.to_string(),
        item("mExplore", "Explore Folder", "E", "Control"),
        item("mCmd", "Command Prompt Here", "P", "Control"),
        item("mCopyPath", "Copy Path", "C", "Control,Menu"),
        SEP.to_string(),
        item("mDelete", "Delete (to Recycle Bin)", "Delete", ""),
        item("mDeleteForever", "Delete (No undelete possible)", "Delete", "Shift"),
        SEP.to_string(),
        item("mExportCsv", "Export to .CSV file...", "E", "Control,Menu"),
        item("mCopyInfo", "Copy file and size info to clipboard", "C", "Control,Shift"),
        item("mExportTypes", "Export File Types to CSV file...", "F", "Control,Menu"),
        SEP.to_string(),
        item("mExit", "Exit", "F4", "Menu"),
    ]
    .concat();
    let options_menu = [
        radio("uDyn", "Display Dynamic values", true),
        radio("uB", "Display Byte values", false),
        radio("uKB", "Display KB values", false),
        radio("uMB", "Display MB values", false),
        radio("uGB", "Display GB values", false),
        radio("uTB", "Display TB values", false),
        SEP.to_string(),
        toggle("oAuto", "Auto scan after making selection", true, "", ""),
        SEP.to_string(),
        toggle("oHeader", "Show Header", true, "H", "Control"),
        toggle("oTypes", "Show File Types", true, "F8", ""),
        toggle("oTreemap", "Show Treemap", true, "F9", ""),
        toggle("oAlloc", "Show Allocated Size on Treemap", false, "", ""),
        toggle("oNames", "Show Folder and File Names on Treemap", true, "N", "Control,Shift"),
        toggle("oTilt", "3D Treemap", true, "T", "Control,Shift"),
        toggle("oCushion", "Cushion shading on Treemap", true, "", ""),
        toggle("oFree", "Show Free Space on Treemap", false, "", ""),
        toggle("oMirror", "Mirror Floor under 3D Treemap", true, "", ""),
        item("mTweaks", "Colors and Tweaks...", "T", "Control"),
        SEP.to_string(),
        item("mAdmin", "Restart as Administrator", "", ""),
        toggle("oAlwaysAdmin", "Always Run as Administrator", false, "", ""),
        toggle("oExplorer", "Add &quot;Scan with DiskTree&quot; to Explorer", false, "", ""),
    ]
    .concat();
    let hud = hud();
    let palettes: String = crate::paint::PALETTES.iter().map(|p| format!(r##"<ComboBoxItem Content="{p}"/>"##)).collect();
    let tweaks = format!(r##"
    <Border x:Name="Tweaks" HorizontalAlignment="Left" VerticalAlignment="Top" Margin="14" Width="300" CornerRadius="10" Padding="16,12,16,14" BorderBrush="#26FFFFFF" BorderThickness="1" Visibility="Collapsed">
      <Border.Background><AcrylicBrush TintColor="#FF141620" TintOpacity="0.55" TintLuminosityOpacity="0.7" FallbackColor="#E8161822"/></Border.Background>
      <StackPanel Spacing="4">
        <Grid>
          <TextBlock Text="Colors and Tweaks" FontSize="13" FontWeight="SemiBold" VerticalAlignment="Center"/>
          <Button x:Name="TweaksClose" HorizontalAlignment="Right" Padding="6" Background="Transparent" BorderThickness="0"><FontIcon Glyph="&#xE711;" FontSize="11"/></Button>
        </Grid>
        <TextBlock Text="Colors" FontSize="11" Foreground="#FF8E97AA" Margin="0,6,0,0"/>
        <ComboBox x:Name="PaletteBox" HorizontalAlignment="Stretch" SelectedIndex="0">{palettes}</ComboBox>
        <TextBlock Text="Tile height when hovered" FontSize="11" Foreground="#FF8E97AA" Margin="0,8,0,0"/>
        <Slider x:Name="LiftSlider" Minimum="0" Maximum="250" Value="100" StepFrequency="5"/>
        <TextBlock Text="Cushion shading" FontSize="11" Foreground="#FF8E97AA"/>
        <Slider x:Name="CushionSlider" Minimum="0" Maximum="200" Value="100" StepFrequency="5"/>
        <TextBlock Text="Tilt" FontSize="11" Foreground="#FF8E97AA"/>
        <Slider x:Name="TiltSlider" Minimum="0" Maximum="100" Value="55" StepFrequency="1"/>
        <ToggleSwitch x:Name="AnimSwitch" Header="Animations" IsOn="True" FontSize="12"/>
      </StackPanel>
    </Border>"##);

    format!(
        r##"<Grid {NS} x:Name="Root" RequestedTheme="Dark" Background="Transparent">
  <Grid.RowDefinitions>
    <RowDefinition Height="40"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition Height="1.5*"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition x:Name="MapRow" Height="1.25*"/>
    <RowDefinition Height="28"/>
  </Grid.RowDefinitions>

  <Grid x:Name="TitleBar" Background="Transparent">
    <StackPanel Orientation="Horizontal" VerticalAlignment="Center" Margin="14,0,0,0" Spacing="10">
      <Border Width="18" Height="18" CornerRadius="5">
        <Border.Background>
          <LinearGradientBrush StartPoint="0,0" EndPoint="1,1">
            <GradientStop Color="#FFFFB347" Offset="0"/><GradientStop Color="#FFFF5E62" Offset="0.55"/><GradientStop Color="#FF8E54E9" Offset="1"/>
          </LinearGradientBrush>
        </Border.Background>
      </Border>
      <TextBlock x:Name="TitleText" Text="DiskTree" FontSize="12" VerticalAlignment="Center" Foreground="#FFC9CFDC"/>
    </StackPanel>
  </Grid>

  <MenuBar Grid.Row="1" Margin="6,0,0,0">
    <MenuBarItem Title="File">{file_menu}</MenuBarItem>
    <MenuBarItem Title="Options">{options_menu}</MenuBarItem>
  </MenuBar>

  <Grid x:Name="Header" Grid.Row="2" Margin="12,4,12,8" ColumnSpacing="10">
    <Grid.ColumnDefinitions>
      <ColumnDefinition Width="380"/>
      <ColumnDefinition Width="*"/>
      <ColumnDefinition Width="330"/>
    </Grid.ColumnDefinitions>
    <Border {CARD} Padding="12,10">
      <StackPanel Spacing="8">
        <Grid ColumnSpacing="8">
          <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
          <TextBlock Text="Select:" VerticalAlignment="Center" Foreground="#FFB9C2D6"/>
          <ComboBox x:Name="DriveBox" Grid.Column="1" HorizontalAlignment="Stretch" SelectedIndex="0">{drive_items}</ComboBox>
          <Button x:Name="ScanButton" Grid.Column="2" Style="{{StaticResource AccentButtonStyle}}" MinWidth="78">
            <StackPanel Orientation="Horizontal" Spacing="6"><FontIcon Glyph="&#xE721;" FontSize="13"/><TextBlock Text="Scan"/></StackPanel>
          </Button>
        </Grid>
        <TextBlock x:Name="StatusText" Text="Ready. Pick a drive or folder and press Scan." FontSize="12" Foreground="#FFB9C2D6"/>
        <ProgressBar x:Name="Progress" Value="0" Maximum="100" Height="4" CornerRadius="2"/>
      </StackPanel>
    </Border>
    <Border Grid.Column="1" {CARD} Padding="14,10">
      <Grid ColumnSpacing="14" RowSpacing="3">
        <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/></Grid.ColumnDefinitions>
        <Grid.RowDefinitions><RowDefinition/><RowDefinition/><RowDefinition/><RowDefinition/><RowDefinition/></Grid.RowDefinitions>
        <TextBlock Text="Selection:" Foreground="#FF8E97AA" FontSize="12"/>
        <TextBlock Text="Total Space:" Grid.Row="1" Foreground="#FF8E97AA" FontSize="12"/>
        <TextBlock Text="Space Used:" Grid.Row="2" Foreground="#FF8E97AA" FontSize="12"/>
        <TextBlock Text="Space Free:" Grid.Row="3" Foreground="#FF8E97AA" FontSize="12"/>
        <TextBlock x:Name="SelText" Grid.Column="1" Grid.ColumnSpan="2" Text="-" FontWeight="SemiBold" FontSize="12" TextTrimming="CharacterEllipsis"/>
        <TextBlock x:Name="TotalText" Grid.Row="1" Grid.Column="1" Text="-" FontWeight="SemiBold" FontSize="12"/>
        <TextBlock x:Name="UsedText" Grid.Row="2" Grid.Column="1" Text="-" FontWeight="SemiBold" FontSize="12"/>
        <TextBlock x:Name="FreeText" Grid.Row="3" Grid.Column="1" Text="-" FontWeight="SemiBold" FontSize="12"/>
        <Grid Grid.Row="1" Grid.RowSpan="3" Grid.Column="2" VerticalAlignment="Center" Margin="10,0,0,0">
          <Border x:Name="UsageTrack" Height="10" CornerRadius="5" Background="#14FFFFFF"/>
          <Border x:Name="UsageBar" Height="10" CornerRadius="5" HorizontalAlignment="Left" Width="0">
            <Border.Background>
              <LinearGradientBrush StartPoint="0,0" EndPoint="1,0">
                <GradientStop Color="#FF4CC2FF" Offset="0"/><GradientStop Color="#FF9B7BFF" Offset="0.8"/><GradientStop Color="#FFFF6B8B" Offset="1"/>
              </LinearGradientBrush>
            </Border.Background>
          </Border>
        </Grid>
        <TextBlock x:Name="ScanInfo" Grid.Row="4" Grid.ColumnSpan="3" Text="" FontSize="11" Foreground="#FF8E97AA"/>
      </Grid>
    </Border>
    <Border Grid.Column="2" {CARD} Padding="14,10">
      <StackPanel Orientation="Horizontal" Spacing="14" VerticalAlignment="Center">
        <Border Width="54" Height="54" CornerRadius="13">
          <Border.Background>
            <LinearGradientBrush StartPoint="0,0" EndPoint="1,1">
              <GradientStop Color="#FFFFB347" Offset="0"/><GradientStop Color="#FFFF5E62" Offset="0.55"/><GradientStop Color="#FF8E54E9" Offset="1"/>
            </LinearGradientBrush>
          </Border.Background>
          <FontIcon Glyph="&#xE9F9;" FontSize="26" Foreground="White"/>
        </Border>
        <StackPanel VerticalAlignment="Center" Spacing="2">
          <TextBlock Text="DiskTree 1.0 x64" FontSize="17" FontWeight="SemiBold"/>
          <TextBlock Text="Rust + WinUI 3 disk space analyser" FontSize="12" Foreground="#FF8E97AA"/>
          <TextBlock x:Name="BrandLine" Text="Hover the map, double-click to zoom" FontSize="11" Foreground="#FF6F788C"/>
        </StackPanel>
      </StackPanel>
    </Border>
  </Grid>

  <Grid Grid.Row="3" Margin="12,0,12,6">
  <TextBox x:Name="FilterBox" HorizontalAlignment="Right" Width="340" Height="32" PlaceholderText="Filter: *.mp4;  node_modules;  *\cache\*" FontSize="12"/>
  <StackPanel Orientation="Horizontal" Spacing="4">
    <Button x:Name="TabTree" Content="Tree View" Padding="14,4" CornerRadius="6" Background="#FF2C3550" BorderThickness="0"/>
    <Button x:Name="TabFiles" Content="File View" Padding="14,4" CornerRadius="6" Background="Transparent" BorderThickness="0"/>
    <Button x:Name="TabAbout" Content="About" Padding="14,4" CornerRadius="6" Background="Transparent" BorderThickness="0"/>
  </StackPanel>
  </Grid>

  <Grid Grid.Row="4" Margin="12,0,12,0" ColumnSpacing="10">
    <Grid.ColumnDefinitions>
      <ColumnDefinition Width="1.4*"/>
      <ColumnDefinition x:Name="TypesCol" Width="1*"/>
    </Grid.ColumnDefinitions>
    <Border {PANE}>
      <Grid>
        <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions>
        <Border x:Name="TreeHeader"/>
        <ScrollViewer x:Name="TreeScroll" Grid.Row="1" HorizontalScrollBarVisibility="Disabled" IsTabStop="True" UseSystemFocusVisuals="False">
          <FlyoutBase.AttachedFlyout>
            <MenuFlyout x:Name="RowMenu">
              <MenuFlyoutItem x:Name="cOpen" Text="Open"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE8E5;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cExplore" Text="Show in Explorer"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xEC50;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cCmd" Text="Command Prompt Here"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE756;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cCopyPath" Text="Copy Path"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE8C8;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutSeparator/>
              <MenuFlyoutItem x:Name="cMapHere" Text="Show on Treemap"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE80A;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cExpandAll" Text="Expand All"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE8A9;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cCopyInfo" Text="Copy Name and Size"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE16F;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutSeparator/>
              <MenuFlyoutItem x:Name="cDelete" Text="Delete (to Recycle Bin)"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE74D;"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
              <MenuFlyoutItem x:Name="cDeleteForever" Text="Delete Permanently"><MenuFlyoutItem.Icon><FontIcon Glyph="&#xE74D;" Foreground="#FFFF6B6B"/></MenuFlyoutItem.Icon></MenuFlyoutItem>
            </MenuFlyout>
          </FlyoutBase.AttachedFlyout>
        </ScrollViewer>
        <ScrollViewer x:Name="AboutPanel" Grid.RowSpan="2" Visibility="Collapsed" Padding="24">
          <StackPanel Spacing="10" MaxWidth="640" HorizontalAlignment="Left">
            <TextBlock Text="DiskTree" FontSize="26" FontWeight="SemiBold"/>
            <TextBlock TextWrapping="Wrap" Foreground="#FFB9C2D6" Text="Scans a drive or folder and shows which folders, files and file types use the most space: as a sortable tree, a list of the biggest files, a table of file types and a lit 3D treemap."/>
            <TextBlock TextWrapping="Wrap" Foreground="#FF8E97AA" Text="Treemap: hover to lift tiles, click to find a file in the tree, double-click or scroll up to zoom into a folder, scroll down to zoom out. Click a file type to light up its files."/>
            <TextBlock TextWrapping="Wrap" Foreground="#FF8E97AA" Text="Shortcuts: F5 scan, F3 pick a folder, F4 collapse, Shift+F4 expand all, F6 parent, F10/F11 zoom the treemap, F8/F9 hide panels, Ctrl+S save the treemap picture, Ctrl+Shift+T flat or 3D."/>
          </StackPanel>
        </ScrollViewer>
      </Grid>
    </Border>
    <Border x:Name="TypesPanel" Grid.Column="1" {PANE}>
      <Grid>
        <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions>
        <Border x:Name="ExtHeader"/>
        <ScrollViewer x:Name="ExtScroll" Grid.Row="1" HorizontalScrollBarVisibility="Disabled"/>
      </Grid>
    </Border>
  </Grid>

  <TextBlock x:Name="MapPath" Grid.Row="5" Text=" " HorizontalAlignment="Center" FontSize="12" Margin="0,7,0,5" Foreground="#FFB9C2D6" TextTrimming="CharacterEllipsis"/>
  <Grid x:Name="MapBorder" Grid.Row="6" Margin="12,0,12,0" CornerRadius="8" BorderBrush="#18FFFFFF" BorderThickness="1" Background="#38000000">
    <SwapChainPanel x:Name="MapPanel"/>
    {hud}
    {tweaks}
  </Grid>

  <Grid Grid.Row="7" Margin="16,0,16,0">
    <TextBlock x:Name="FootLeft" Text="" FontSize="11" Foreground="#FF8E97AA" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"/>
    <TextBlock x:Name="FootRight" Text="" FontSize="11" Foreground="#FF8E97AA" VerticalAlignment="Center" HorizontalAlignment="Right"/>
  </Grid>
</Grid>"##
    )
}
