package xyz.mendess.jukebox

import android.os.Bundle
import androidx.appcompat.app.AppCompatActivity
import androidx.fragment.app.Fragment
import androidx.viewpager.widget.ViewPager
import com.google.android.material.tabs.TabLayout
import xyz.mendess.jukebox.ui.main.Home
import xyz.mendess.jukebox.ui.main.SectionsPagerAdapter

class MainActivity : AppCompatActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        val tabs: TabLayout = findViewById(R.id.tabs)
        val viewPager: ViewPager = findViewById(R.id.view_pager)
        Thread { JukeboxLib.startUserThread("192.168.1.21:4192") }.start()
        val fragments: Array<Fragment> = arrayOf(
            Home.newInstance(),
            Home.newInstance(),
            Home.newInstance()
        )
        viewPager.adapter = SectionsPagerAdapter(this, supportFragmentManager, fragments)
        tabs.setupWithViewPager(viewPager)
    }

    companion object {
        init {
            System.loadLibrary("jukebox")
        }
    }

}

