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

/// Column layout of the tree table: name, % of parent, size, allocated, items, files.
pub const TREE_COLS: &str = r##"<Grid.ColumnDefinitions><ColumnDefinition Width="*"/><ColumnDefinition Width="118"/><ColumnDefinition Width="92"/><ColumnDefinition Width="92"/><ColumnDefinition Width="84"/><ColumnDefinition Width="84"/></Grid.ColumnDefinitions>"##;
/// File view: name, size, allocated, modified, folder.
pub const FILE_COLS: &str = r##"<Grid.ColumnDefinitions><ColumnDefinition Width="2*"/><ColumnDefinition Width="92"/><ColumnDefinition Width="92"/><ColumnDefinition Width="130"/><ColumnDefinition Width="3*"/></Grid.ColumnDefinitions>"##;
/// Extension table: swatch, extension, file type, percent, size, allocated, files.
pub const EXT_COLS: &str = r##"<Grid.ColumnDefinitions><ColumnDefinition Width="22"/><ColumnDefinition Width="78"/><ColumnDefinition Width="*"/><ColumnDefinition Width="96"/><ColumnDefinition Width="80"/><ColumnDefinition Width="80"/><ColumnDefinition Width="70"/></Grid.ColumnDefinitions>"##;

pub fn header_cells(cols: &str, names: &[(&str, bool)]) -> String {
    let mut s = format!(r##"<Grid {NS} Height="30" Background="#FF1F2129">{cols}"##);
    for (i, (n, right)) in names.iter().enumerate() {
        let align = if *right { "Right" } else { "Left" };
        s += &format!(
            r##"<TextBlock Grid.Column="{i}" Text="{}" FontSize="12" FontWeight="SemiBold" Foreground="#FFB9C2D6" VerticalAlignment="Center" HorizontalAlignment="{align}" Margin="10,0,10,0"/>"##,
            esc(n)
        );
    }
    s += r##"<Border Grid.ColumnSpan="9" Height="1" VerticalAlignment="Bottom" Background="#FF34384A"/></Grid>"##;
    s
}

pub fn page(drives: &[String]) -> String {
    let drive_items: String = drives
        .iter()
        .map(|d| format!(r##"<ComboBoxItem Content="{}"/>"##, esc(d)))
        .collect();
    let file_menu = [
        item("mScan", "Scan", "F5", ""),
        item("mSelectFolder", "Select Folder...", "F3", ""),
        item("mNewInstance", "New Instance", "N", "Control"),
        SEP.to_string(),
        item("mReset", "Reset tree expansion", "F4", ""),
        item("mExpandAll", "Expand All", "F4", "Shift"),
        item("mParent", "Select Parent", "F6", ""),
        item("mZoomIn", "Zoom in Treemap", "F10", ""),
        item("mZoomOut", "Zoom out Treemap", "F11", ""),
        item("mSavePng", "Save Treemap Image as .png file", "S", "Control"),
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
        toggle("oCushion", "Cushion shading on Treemap", true, "", ""),
    ]
    .concat();

    format!(
        r##"<Grid {NS} x:Name="Root" RequestedTheme="Dark">
  <Grid.Background>
    <LinearGradientBrush StartPoint="0,0" EndPoint="1,1">
      <GradientStop Color="#FF14151B" Offset="0"/>
      <GradientStop Color="#FF181B24" Offset="0.6"/>
      <GradientStop Color="#FF151820" Offset="1"/>
    </LinearGradientBrush>
  </Grid.Background>
  <Grid.RowDefinitions>
    <RowDefinition Height="36"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition Height="1.85*"/>
    <RowDefinition Height="Auto"/>
    <RowDefinition x:Name="MapRow" Height="1*"/>
    <RowDefinition Height="28"/>
  </Grid.RowDefinitions>

  <!-- Title bar -->
  <Grid x:Name="TitleBar" Background="Transparent">
    <StackPanel Orientation="Horizontal" VerticalAlignment="Center" Margin="12,0,0,0" Spacing="10">
      <Border Width="20" Height="20" CornerRadius="5">
        <Border.Background>
          <LinearGradientBrush StartPoint="0,0" EndPoint="1,1">
            <GradientStop Color="#FFFFB347" Offset="0"/>
            <GradientStop Color="#FFFF5E62" Offset="1"/>
          </LinearGradientBrush>
        </Border.Background>
        <FontIcon Glyph="&#xE8B7;" FontSize="11" Foreground="White"/>
      </Border>
      <TextBlock x:Name="TitleText" Text="DiskTree" FontSize="12" VerticalAlignment="Center" Foreground="#FFD0D4DE"/>
    </StackPanel>
  </Grid>

  <!-- Menu -->
  <MenuBar Grid.Row="1" Margin="4,0,0,0">
    <MenuBarItem Title="File">{file_menu}</MenuBarItem>
    <MenuBarItem Title="Options">{options_menu}</MenuBarItem>
  </MenuBar>

  <!-- Header cards -->
  <Grid x:Name="Header" Grid.Row="2" Margin="10,4,10,6" ColumnSpacing="10">
    <Grid.ColumnDefinitions>
      <ColumnDefinition Width="380"/>
      <ColumnDefinition Width="*"/>
      <ColumnDefinition Width="330"/>
    </Grid.ColumnDefinitions>
    <Border CornerRadius="8" Background="#FF1E2029" BorderBrush="#FF2E3240" BorderThickness="1" Padding="12,10">
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
        <ProgressBar x:Name="Progress" Value="0" Maximum="100" Height="6" CornerRadius="3"/>
      </StackPanel>
    </Border>
    <Border Grid.Column="1" CornerRadius="8" Background="#FF1E2029" BorderBrush="#FF2E3240" BorderThickness="1" Padding="14,10">
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
          <Border x:Name="UsageTrack" Height="14" CornerRadius="7" Background="#FF2A2E3A"/>
          <Border x:Name="UsageBar" Height="14" CornerRadius="7" HorizontalAlignment="Left" Width="0">
            <Border.Background>
              <LinearGradientBrush StartPoint="0,0" EndPoint="1,0">
                <GradientStop Color="#FF3A7BD5" Offset="0"/>
                <GradientStop Color="#FF00D2FF" Offset="0.7"/>
                <GradientStop Color="#FFFF6B6B" Offset="1"/>
              </LinearGradientBrush>
            </Border.Background>
          </Border>
        </Grid>
        <TextBlock x:Name="ScanInfo" Grid.Row="4" Grid.ColumnSpan="3" Text="" FontSize="11" Foreground="#FF8E97AA"/>
      </Grid>
    </Border>
    <Border Grid.Column="2" CornerRadius="8" BorderBrush="#FF2E3240" BorderThickness="1" Padding="14,10">
      <Border.Background>
        <LinearGradientBrush StartPoint="0,0" EndPoint="1,1">
          <GradientStop Color="#FF232637" Offset="0"/>
          <GradientStop Color="#FF1B1D27" Offset="1"/>
        </LinearGradientBrush>
      </Border.Background>
      <StackPanel Orientation="Horizontal" Spacing="14" VerticalAlignment="Center">
        <Border Width="54" Height="54" CornerRadius="12">
          <Border.Background>
            <LinearGradientBrush StartPoint="0,0" EndPoint="1,1">
              <GradientStop Color="#FFFFB347" Offset="0"/>
              <GradientStop Color="#FFFF5E62" Offset="0.55"/>
              <GradientStop Color="#FF8E54E9" Offset="1"/>
            </LinearGradientBrush>
          </Border.Background>
          <FontIcon Glyph="&#xE9F9;" FontSize="26" Foreground="White"/>
        </Border>
        <StackPanel VerticalAlignment="Center" Spacing="2">
          <TextBlock Text="DiskTree 1.0 x64" FontSize="17" FontWeight="SemiBold"/>
          <TextBlock Text="Rust + WinUI 3 disk space analyser" FontSize="12" Foreground="#FF8E97AA"/>
          <TextBlock Text="Find what fills your drive at a glance" FontSize="11" Foreground="#FF6F788C"/>
        </StackPanel>
      </StackPanel>
    </Border>
  </Grid>

  <!-- View tabs -->
  <StackPanel Grid.Row="3" Orientation="Horizontal" Margin="10,0,10,4" Spacing="4">
    <Button x:Name="TabTree" Content="Tree View" Padding="14,4" CornerRadius="6" Background="#FF2C3550" BorderThickness="0"/>
    <Button x:Name="TabFiles" Content="File View" Padding="14,4" CornerRadius="6" Background="Transparent" BorderThickness="0"/>
    <Button x:Name="TabAbout" Content="About" Padding="14,4" CornerRadius="6" Background="Transparent" BorderThickness="0"/>
  </StackPanel>

  <!-- Tables -->
  <Grid Grid.Row="4" Margin="10,0,10,0" ColumnSpacing="10">
    <Grid.ColumnDefinitions>
      <ColumnDefinition Width="1.4*"/>
      <ColumnDefinition x:Name="TypesCol" Width="1*"/>
    </Grid.ColumnDefinitions>
    <Border CornerRadius="8" Background="#FF1A1C24" BorderBrush="#FF2E3240" BorderThickness="1">
      <Grid>
        <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions>
        <Border x:Name="TreeHeader"/>
        <ScrollViewer x:Name="TreeScroll" Grid.Row="1" HorizontalScrollBarVisibility="Disabled"/>
        <ScrollViewer x:Name="AboutPanel" Grid.RowSpan="2" Visibility="Collapsed" Padding="24">
          <StackPanel Spacing="10" MaxWidth="640" HorizontalAlignment="Left">
            <TextBlock Text="DiskTree" FontSize="26" FontWeight="SemiBold"/>
            <TextBlock TextWrapping="Wrap" Foreground="#FFB9C2D6" Text="Scans a drive or folder and shows which folders, files and file types use the most space: as a sortable tree, a list of the biggest files, a table of file types and a cushion-shaded treemap."/>
            <TextBlock TextWrapping="Wrap" Foreground="#FF8E97AA" Text="Click a folder to select it, double-click (or click the arrow) to open it. Click a tile in the treemap to find its file in the tree. Click a file type to light up its files in the treemap."/>
            <TextBlock TextWrapping="Wrap" Foreground="#FF8E97AA" Text="Shortcuts: F5 scan, F3 pick a folder, F4 collapse, Shift+F4 expand all, F6 parent, F10/F11 zoom the treemap, F8/F9 hide panels, Ctrl+S save the treemap picture."/>
          </StackPanel>
        </ScrollViewer>
      </Grid>
    </Border>
    <Border x:Name="TypesPanel" Grid.Column="1" CornerRadius="8" Background="#FF1A1C24" BorderBrush="#FF2E3240" BorderThickness="1">
      <Grid>
        <Grid.RowDefinitions><RowDefinition Height="Auto"/><RowDefinition Height="*"/></Grid.RowDefinitions>
        <Border x:Name="ExtHeader"/>
        <ScrollViewer x:Name="ExtScroll" Grid.Row="1" HorizontalScrollBarVisibility="Disabled"/>
      </Grid>
    </Border>
  </Grid>

  <!-- Treemap -->
  <TextBlock x:Name="MapPath" Grid.Row="5" Text=" " HorizontalAlignment="Center" FontSize="12" Margin="0,6,0,4" Foreground="#FFB9C2D6" TextTrimming="CharacterEllipsis"/>
  <Border x:Name="MapBorder" Grid.Row="6" Margin="10,0,10,0" CornerRadius="8" BorderBrush="#FF2E3240" BorderThickness="1" Background="#FF111217" Padding="2">
    <Image x:Name="MapImage" Stretch="Fill"/>
  </Border>

  <!-- Status bar -->
  <Grid Grid.Row="7" Margin="14,0,14,0">
    <TextBlock x:Name="FootLeft" Text="" FontSize="11" Foreground="#FF8E97AA" VerticalAlignment="Center" TextTrimming="CharacterEllipsis"/>
    <TextBlock x:Name="FootRight" Text="" FontSize="11" Foreground="#FF8E97AA" VerticalAlignment="Center" HorizontalAlignment="Right"/>
  </Grid>
</Grid>"##
    )
}
